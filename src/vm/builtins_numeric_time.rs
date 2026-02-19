use super::{
    BigInt, Duration, HashMap, Instant, MONOTONIC_START, Object, RuntimeError, SystemTime,
    UNIX_EPOCH, Value, Vm, erfc_approx, format_strftime, random_range_count, seed_from_value,
    split_unix_timestamp, time_parts_from_value, unix_seconds_now, value_from_bigint,
    value_to_bigint, value_to_f64, value_to_int,
};

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
#[link(name = "m")]
unsafe extern "C" {
    fn lgamma(x: f64) -> f64;
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
unsafe extern "C" {
    fn lgamma(x: f64) -> f64;
}

#[cfg(unix)]
fn native_lgamma(x: f64) -> f64 {
    // SAFETY: libc `lgamma` is pure for finite inputs and has no side effects.
    unsafe { lgamma(x) }
}

#[cfg(not(unix))]
fn native_lgamma(x: f64) -> f64 {
    // Fallback approximation for non-Unix targets.
    if x <= 0.0 {
        return f64::NAN;
    }
    let pi = std::f64::consts::PI;
    let z = x;
    (z - 0.5) * z.ln() - z + 0.5 * (2.0 * pi).ln()
}

impl Vm {
    pub(super) fn builtin_random_seed(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.strip_random_self_arg(&mut args);
        if args.len() > 1 {
            return Err(RuntimeError::new("seed() takes at most 1 argument"));
        }
        let kw_value = kwargs.remove("a");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "seed() got an unexpected keyword argument",
            ));
        }
        if kw_value.is_some() && !args.is_empty() {
            return Err(RuntimeError::new(
                "seed() got multiple values for argument 'a'",
            ));
        }
        let value = kw_value.or_else(|| args.pop()).unwrap_or(Value::None);
        let seed = seed_from_value(&value)?;
        self.random.seed(seed);
        Ok(Value::None)
    }

    pub(super) fn builtin_random_random(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.strip_random_self_arg(&mut args);
        if !args.is_empty() || !kwargs.is_empty() {
            return Err(RuntimeError::new("random() takes no arguments"));
        }
        Ok(Value::Float(self.random.random_f64()))
    }

    pub(super) fn builtin_random_randrange(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.strip_random_self_arg(&mut args);
        if args.len() > 3 {
            return Err(RuntimeError::new(
                "randrange() expected at most 3 arguments",
            ));
        }
        let mut start_kw = kwargs.remove("start");
        let mut stop_kw = kwargs.remove("stop");
        let mut step_kw = kwargs.remove("step");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "randrange() got an unexpected keyword argument",
            ));
        }

        match args.len() {
            0 => {}
            1 => {
                if stop_kw.is_some() {
                    return Err(RuntimeError::new("randrange() got multiple values"));
                }
                stop_kw = Some(args.remove(0));
            }
            2 => {
                if start_kw.is_some() || stop_kw.is_some() {
                    return Err(RuntimeError::new("randrange() got multiple values"));
                }
                start_kw = Some(args.remove(0));
                stop_kw = Some(args.remove(0));
            }
            3 => {
                if start_kw.is_some() || stop_kw.is_some() || step_kw.is_some() {
                    return Err(RuntimeError::new("randrange() got multiple values"));
                }
                start_kw = Some(args.remove(0));
                stop_kw = Some(args.remove(0));
                step_kw = Some(args.remove(0));
            }
            _ => unreachable!(),
        }

        let stop = stop_kw.ok_or_else(|| RuntimeError::new("randrange() missing stop"))?;
        let start = start_kw.unwrap_or(Value::Int(0));
        let step = step_kw.unwrap_or(Value::Int(1));

        let start = value_to_int(start)?;
        let stop = value_to_int(stop)?;
        let step = value_to_int(step)?;
        if step == 0 {
            return Err(RuntimeError::new(
                "randrange() step argument must not be zero",
            ));
        }

        let count = random_range_count(start, stop, step)?;
        let offset = self.random_randbelow(count)?;
        let result = (start as i128)
            .checked_add((step as i128) * (offset as i128))
            .ok_or_else(|| RuntimeError::new("integer overflow"))?;
        let result = i64::try_from(result).map_err(|_| RuntimeError::new("integer overflow"))?;
        Ok(Value::Int(result))
    }

    pub(super) fn builtin_random_randint(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.strip_random_self_arg(&mut args);
        if args.len() > 2 {
            return Err(RuntimeError::new("randint() expected 2 arguments"));
        }
        let a_kw = kwargs.remove("a");
        let b_kw = kwargs.remove("b");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "randint() got an unexpected keyword argument",
            ));
        }

        let a_value = if let Some(value) = a_kw {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "randint() got multiple values for argument 'a'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new("randint() missing argument 'a'"));
        };
        let b_value = if let Some(value) = b_kw {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "randint() got multiple values for argument 'b'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new("randint() missing argument 'b'"));
        };
        if !args.is_empty() {
            return Err(RuntimeError::new("randint() expected 2 arguments"));
        }

        let a = value_to_int(a_value)?;
        let b = value_to_int(b_value)?;
        let upper = b
            .checked_add(1)
            .ok_or_else(|| RuntimeError::new("empty range for randint()"))?;
        let count = random_range_count(a, upper, 1)?;
        let offset = self.random_randbelow(count)?;
        let result = a
            .checked_add(offset)
            .ok_or_else(|| RuntimeError::new("integer overflow"))?;
        Ok(Value::Int(result))
    }

    pub(super) fn builtin_random_getrandbits(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.strip_random_self_arg(&mut args);
        if args.len() > 1 {
            return Err(RuntimeError::new(
                "getrandbits() takes exactly one argument",
            ));
        }
        let kw_k = kwargs.remove("k");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "getrandbits() got an unexpected keyword argument",
            ));
        }
        let k_value = if let Some(value) = kw_k {
            if !args.is_empty() {
                return Err(RuntimeError::new(
                    "getrandbits() got multiple values for 'k'",
                ));
            }
            value
        } else if !args.is_empty() {
            args.remove(0)
        } else {
            return Err(RuntimeError::new("getrandbits() missing argument 'k'"));
        };

        let bits = value_to_int(k_value)?;
        if bits < 0 {
            return Err(RuntimeError::new("number of bits must be non-negative"));
        }
        if bits == 0 {
            return Ok(Value::Int(0));
        }
        if bits > 63 {
            return Err(RuntimeError::new(
                "getrandbits() supports up to 63 bits in this runtime",
            ));
        }

        let mut produced = 0u64;
        let mut consumed = 0i64;
        while consumed < bits {
            let chunk = self.random.next_u32() as u64;
            let take = std::cmp::min(32, (bits - consumed) as usize);
            let mask = if take == 32 {
                u64::MAX
            } else {
                (1u64 << take) - 1
            };
            produced |= (chunk & mask) << consumed;
            consumed += take as i64;
        }
        Ok(Value::Int(produced as i64))
    }

    pub(super) fn builtin_random_choice(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.strip_random_self_arg(&mut args);
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("choice() expects one argument"));
        }
        match &args[0] {
            Value::List(obj) => match &*obj.kind() {
                Object::List(values) => {
                    if values.is_empty() {
                        return Err(RuntimeError::new("Cannot choose from an empty sequence"));
                    }
                    let idx = self.random_randbelow(values.len() as i64)? as usize;
                    Ok(values[idx].clone())
                }
                _ => Err(RuntimeError::new("choice() expects a sequence")),
            },
            Value::Tuple(obj) => match &*obj.kind() {
                Object::Tuple(values) => {
                    if values.is_empty() {
                        return Err(RuntimeError::new("Cannot choose from an empty sequence"));
                    }
                    let idx = self.random_randbelow(values.len() as i64)? as usize;
                    Ok(values[idx].clone())
                }
                _ => Err(RuntimeError::new("choice() expects a sequence")),
            },
            Value::Str(value) => {
                let chars: Vec<char> = value.chars().collect();
                if chars.is_empty() {
                    return Err(RuntimeError::new("Cannot choose from an empty sequence"));
                }
                let idx = self.random_randbelow(chars.len() as i64)? as usize;
                Ok(Value::Str(chars[idx].to_string()))
            }
            _ => Err(RuntimeError::new("choice() expects a sequence")),
        }
    }

    pub(super) fn builtin_random_choices(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.strip_random_self_arg(&mut args);
        if args.is_empty() {
            return Err(RuntimeError::new(
                "choices() missing required argument 'population'",
            ));
        }
        if args.len() > 2 {
            return Err(RuntimeError::new(
                "choices() expected at most 2 positional arguments",
            ));
        }
        let population_source = args.remove(0);
        let population = match population_source {
            Value::Str(text) => text
                .chars()
                .map(|ch| Value::Str(ch.to_string()))
                .collect::<Vec<_>>(),
            other => self.collect_iterable_values(other)?,
        };
        let mut weights = if !args.is_empty() {
            Some(args.remove(0))
        } else {
            None
        };
        if let Some(value) = kwargs.remove("weights") {
            if weights.is_some() {
                return Err(RuntimeError::new(
                    "choices() got multiple values for argument 'weights'",
                ));
            }
            weights = Some(value);
        }
        let cum_weights = kwargs.remove("cum_weights");
        if weights.is_some() && cum_weights.is_some() {
            return Err(RuntimeError::new(
                "cannot specify both weights and cum_weights",
            ));
        }
        let k_value = kwargs.remove("k").unwrap_or(Value::Int(1));
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "choices() got an unexpected keyword argument",
            ));
        }

        let k = value_to_int(k_value)?;
        if k < 0 {
            return Err(RuntimeError::new("k must be a non-negative integer"));
        }
        let k = k as usize;
        if population.is_empty() {
            if k == 0 {
                return Ok(self.heap.alloc_list(Vec::new()));
            }
            return Err(RuntimeError::new("Cannot choose from an empty sequence"));
        }

        let mut out = Vec::with_capacity(k);
        if let Some(weight_source) = weights.or(cum_weights) {
            let raw = self.collect_iterable_values(weight_source)?;
            if raw.len() != population.len() {
                return Err(RuntimeError::new(
                    "the number of weights does not match the population",
                ));
            }
            let mut cumulative = Vec::with_capacity(raw.len());
            let mut total = 0.0_f64;
            for value in raw {
                let weight = value_to_f64(value)?;
                if !weight.is_finite() || weight < 0.0 {
                    return Err(RuntimeError::new(
                        "weights must be non-negative finite numbers",
                    ));
                }
                total += weight;
                cumulative.push(total);
            }
            if total <= 0.0 {
                return Err(RuntimeError::new(
                    "total of weights must be greater than zero",
                ));
            }
            for _ in 0..k {
                let needle = self.random.random_f64() * total;
                let idx = cumulative
                    .iter()
                    .position(|bound| needle < *bound)
                    .unwrap_or(cumulative.len().saturating_sub(1));
                out.push(population[idx].clone());
            }
        } else {
            for _ in 0..k {
                let idx = self.random_randbelow(population.len() as i64)? as usize;
                out.push(population[idx].clone());
            }
        }
        Ok(self.heap.alloc_list(out))
    }

    pub(super) fn builtin_random_shuffle(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.strip_random_self_arg(&mut args);
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("shuffle() expects one argument"));
        }
        match &args[0] {
            Value::List(obj) => {
                let len = match &*obj.kind() {
                    Object::List(values) => values.len(),
                    _ => return Err(RuntimeError::new("shuffle() expects list")),
                };
                if len <= 1 {
                    return Ok(Value::None);
                }
                for idx in (1..len).rev() {
                    let swap = self.random_randbelow((idx + 1) as i64)? as usize;
                    if let Object::List(values) = &mut *obj.kind_mut() {
                        values.swap(idx, swap);
                    }
                }
                Ok(Value::None)
            }
            _ => Err(RuntimeError::new("shuffle() expects list")),
        }
    }

    pub(super) fn strip_random_self_arg(&self, args: &mut Vec<Value>) {
        let remove_self = matches!(
            args.first(),
            Some(Value::Instance(instance))
                if matches!(
                    &*instance.kind(),
                    Object::Instance(instance_data)
                        if matches!(
                            &*instance_data.class.kind(),
                            Object::Class(class_data)
                                if class_data.name == "Random" || class_data.name == "SystemRandom"
                        )
                )
        );
        if remove_self {
            let _ = args.remove(0);
        }
    }

    pub(super) fn builtin_decimal_getcontext(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("getcontext() expects no arguments"));
        }
        let module = self
            .modules
            .get("decimal")
            .cloned()
            .ok_or_else(|| RuntimeError::new("decimal module unavailable"))?;
        let Object::Module(module_data) = &*module.kind() else {
            return Err(RuntimeError::new("invalid decimal module"));
        };
        Ok(module_data
            .globals
            .get("_context")
            .cloned()
            .unwrap_or(Value::None))
    }

    pub(super) fn builtin_decimal_setcontext(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new(
                "setcontext() expects one context argument",
            ));
        }
        let context = args.remove(0);
        let module = self
            .modules
            .get("decimal")
            .cloned()
            .ok_or_else(|| RuntimeError::new("decimal module unavailable"))?;
        let Object::Module(module_data) = &mut *module.kind_mut() else {
            return Err(RuntimeError::new("invalid decimal module"));
        };
        module_data.globals.insert("_context".to_string(), context);
        Ok(Value::None)
    }

    pub(super) fn builtin_decimal_localcontext(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::new(
                "localcontext() expects at most one argument",
            ));
        }
        let context = if !args.is_empty() {
            args.remove(0)
        } else if let Some(value) = kwargs.remove("ctx") {
            value
        } else {
            self.builtin_decimal_getcontext(Vec::new(), HashMap::new())?
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "localcontext() got an unexpected keyword argument",
            ));
        }
        Ok(context)
    }

    pub(super) fn builtin_math_sqrt(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("sqrt() expects one argument"));
        }
        let value = value_to_f64(args[0].clone())?;
        if value < 0.0 {
            return Err(RuntimeError::new("math domain error"));
        }
        Ok(Value::Float(value.sqrt()))
    }

    pub(super) fn builtin_math_factorial(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("factorial() expects one argument"));
        }
        let mut value = value_to_bigint(args[0].clone())
            .map_err(|_| RuntimeError::new("factorial() only accepts integral values"))?;
        if value.is_negative() {
            return Err(RuntimeError::new(
                "factorial() not defined for negative values",
            ));
        }
        let mut out = BigInt::one();
        let one = BigInt::one();
        while !value.is_zero() {
            out = out.mul(&value);
            value = value.sub(&one);
        }
        Ok(value_from_bigint(out))
    }

    pub(super) fn builtin_math_gcd(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new("gcd() does not accept keyword arguments"));
        }
        if args.is_empty() {
            return Ok(Value::Int(0));
        }

        let mut acc = BigInt::zero();
        for value in args {
            let rhs = value_to_bigint(value)
                .map_err(|_| RuntimeError::new("gcd() only accepts integral values"))?
                .abs();
            if acc.is_zero() {
                acc = rhs;
                continue;
            }

            // Euclidean algorithm on non-negative integers.
            let mut left = acc;
            let mut right = rhs;
            while !right.is_zero() {
                let (_, remainder) = left
                    .div_mod_floor(&right)
                    .ok_or_else(|| RuntimeError::new("integer division by zero"))?;
                left = right;
                right = remainder;
            }
            acc = left;
        }
        Ok(value_from_bigint(acc))
    }

    pub(super) fn builtin_math_copysign(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("copysign() expects two arguments"));
        }
        let magnitude = value_to_f64(args[0].clone())?;
        let sign = value_to_f64(args[1].clone())?;
        Ok(Value::Float(magnitude.copysign(sign)))
    }

    pub(super) fn builtin_math_floor(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("floor() expects one argument"));
        }
        let value = value_to_f64(args[0].clone())?.floor();
        if value < i64::MIN as f64 || value > i64::MAX as f64 {
            return Err(RuntimeError::new("integer overflow"));
        }
        Ok(Value::Int(value as i64))
    }

    pub(super) fn builtin_math_ceil(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("ceil() expects one argument"));
        }
        let value = value_to_f64(args[0].clone())?.ceil();
        if value < i64::MIN as f64 || value > i64::MAX as f64 {
            return Err(RuntimeError::new("integer overflow"));
        }
        Ok(Value::Int(value as i64))
    }

    pub(super) fn builtin_math_trunc(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("trunc() expects one argument"));
        }
        match args[0].clone() {
            Value::Int(_) | Value::BigInt(_) => Ok(args[0].clone()),
            other => {
                let truncated = value_to_f64(other)?.trunc();
                let bigint = BigInt::from_f64_integral(truncated)
                    .ok_or_else(|| RuntimeError::new("cannot convert float to int"))?;
                Ok(value_from_bigint(bigint))
            }
        }
    }

    pub(super) fn builtin_math_isfinite(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isfinite() expects one argument"));
        }
        Ok(Value::Bool(value_to_f64(args[0].clone())?.is_finite()))
    }

    pub(super) fn builtin_math_isinf(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isinf() expects one argument"));
        }
        Ok(Value::Bool(value_to_f64(args[0].clone())?.is_infinite()))
    }

    pub(super) fn builtin_math_isnan(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("isnan() expects one argument"));
        }
        Ok(Value::Bool(value_to_f64(args[0].clone())?.is_nan()))
    }

    pub(super) fn builtin_math_ldexp(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new("ldexp() expects two arguments"));
        }
        let x = value_to_f64(args[0].clone())?;
        let i = value_to_int(args[1].clone())?;
        Ok(Value::Float(x * (2.0f64).powf(i as f64)))
    }

    pub(super) fn builtin_math_hypot(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "hypot() does not accept keyword arguments",
            ));
        }
        if args.is_empty() {
            return Ok(Value::Float(0.0));
        }
        let mut max = 0.0f64;
        let mut sum = 0.0f64;
        let mut saw_nan = false;
        for value in args {
            let abs = value_to_f64(value)?.abs();
            if abs.is_infinite() {
                return Ok(Value::Float(f64::INFINITY));
            }
            if abs.is_nan() {
                saw_nan = true;
                continue;
            }
            if abs > max {
                let ratio = if max == 0.0 { 0.0 } else { max / abs };
                sum = (sum * ratio * ratio) + 1.0;
                max = abs;
            } else if abs != 0.0 {
                let ratio = abs / max;
                sum += ratio * ratio;
            }
        }
        if saw_nan && max == 0.0 {
            return Ok(Value::Float(f64::NAN));
        }
        Ok(Value::Float(max * sum.sqrt()))
    }

    pub(super) fn builtin_math_fabs(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("fabs() expects one argument"));
        }
        Ok(Value::Float(value_to_f64(args[0].clone())?.abs()))
    }

    pub(super) fn builtin_math_exp(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("exp() expects one argument"));
        }
        Ok(Value::Float(value_to_f64(args[0].clone())?.exp()))
    }

    pub(super) fn builtin_math_erfc(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("erfc() expects one argument"));
        }
        Ok(Value::Float(erfc_approx(value_to_f64(args[0].clone())?)))
    }

    pub(super) fn builtin_math_log(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new("log() expects x and optional base"));
        }
        let x = value_to_f64(args[0].clone())?;
        if x <= 0.0 {
            return Err(RuntimeError::new("math domain error"));
        }
        if args.len() == 1 {
            return Ok(Value::Float(x.ln()));
        }
        let base = value_to_f64(args[1].clone())?;
        if base <= 0.0 || base == 1.0 {
            return Err(RuntimeError::new("math domain error"));
        }
        Ok(Value::Float(x.ln() / base.ln()))
    }

    pub(super) fn builtin_math_lgamma(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("lgamma() expects one argument"));
        }
        let x = value_to_f64(args[0].clone())?;
        if x.is_finite() && x <= 0.0 && x.fract() == 0.0 {
            return Err(RuntimeError::new("math domain error"));
        }
        let value = native_lgamma(x);
        if !x.is_nan() && value.is_nan() {
            return Err(RuntimeError::new("math domain error"));
        }
        Ok(Value::Float(value))
    }

    pub(super) fn builtin_math_log2(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("log2() expects one argument"));
        }
        let x = value_to_f64(args[0].clone())?;
        if x <= 0.0 {
            return Err(RuntimeError::new("math domain error"));
        }
        Ok(Value::Float(x.log2()))
    }

    pub(super) fn builtin_math_fsum(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("fsum() expects one iterable argument"));
        }
        let values = self.collect_iterable_values(args[0].clone())?;
        let mut sum = 0.0f64;
        let mut c = 0.0f64;
        for value in values {
            let y = value_to_f64(value)? - c;
            let t = sum + y;
            c = (t - sum) - y;
            sum = t;
        }
        Ok(Value::Float(sum))
    }

    pub(super) fn builtin_math_sumprod(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 2 {
            return Err(RuntimeError::new(
                "sumprod() expects two iterable arguments",
            ));
        }
        let left = self.collect_iterable_values(args[0].clone())?;
        let right = self.collect_iterable_values(args[1].clone())?;
        if left.len() != right.len() {
            return Err(RuntimeError::new(
                "sumprod() inputs are not the same length",
            ));
        }
        let mut sum = 0.0f64;
        let mut c = 0.0f64;
        for (a, b) in left.into_iter().zip(right.into_iter()) {
            let product = value_to_f64(a)? * value_to_f64(b)?;
            let y = product - c;
            let t = sum + y;
            c = (t - sum) - y;
            sum = t;
        }
        Ok(Value::Float(sum))
    }

    pub(super) fn builtin_math_cos(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("cos() expects one argument"));
        }
        Ok(Value::Float(value_to_f64(args[0].clone())?.cos()))
    }

    pub(super) fn builtin_math_sin(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("sin() expects one argument"));
        }
        Ok(Value::Float(value_to_f64(args[0].clone())?.sin()))
    }

    pub(super) fn builtin_math_tan(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("tan() expects one argument"));
        }
        Ok(Value::Float(value_to_f64(args[0].clone())?.tan()))
    }

    pub(super) fn builtin_math_cosh(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("cosh() expects one argument"));
        }
        Ok(Value::Float(value_to_f64(args[0].clone())?.cosh()))
    }

    pub(super) fn builtin_math_asin(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("asin() expects one argument"));
        }
        let x = value_to_f64(args[0].clone())?;
        if !(-1.0..=1.0).contains(&x) {
            return Err(RuntimeError::new("math domain error"));
        }
        Ok(Value::Float(x.asin()))
    }

    pub(super) fn builtin_math_atan(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("atan() expects one argument"));
        }
        Ok(Value::Float(value_to_f64(args[0].clone())?.atan()))
    }

    pub(super) fn builtin_math_acos(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("acos() expects one argument"));
        }
        let x = value_to_f64(args[0].clone())?;
        if !(-1.0..=1.0).contains(&x) {
            return Err(RuntimeError::new("math domain error"));
        }
        Ok(Value::Float(x.acos()))
    }

    pub(super) fn builtin_math_isclose(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != 2 {
            return Err(RuntimeError::new(
                "isclose() expects two positional arguments",
            ));
        }
        let a = value_to_f64(args.remove(0))?;
        let b = value_to_f64(args.remove(0))?;
        let rel_tol = if let Some(value) = kwargs.remove("rel_tol") {
            value_to_f64(value)?
        } else {
            1e-9
        };
        let abs_tol = if let Some(value) = kwargs.remove("abs_tol") {
            value_to_f64(value)?
        } else {
            0.0
        };
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "isclose() got an unexpected keyword argument",
            ));
        }
        if rel_tol < 0.0 || abs_tol < 0.0 {
            return Err(RuntimeError::new("tolerances must be non-negative"));
        }
        if a == b {
            return Ok(Value::Bool(true));
        }
        if a.is_infinite() || b.is_infinite() {
            return Ok(Value::Bool(false));
        }
        let diff = (a - b).abs();
        let tol = (rel_tol * a.abs()).max(rel_tol * b.abs()).max(abs_tol);
        Ok(Value::Bool(diff <= tol))
    }

    pub(super) fn builtin_time_time(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("time() expects no arguments"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RuntimeError::new("system time before epoch"))?;
        Ok(Value::Float(now.as_secs_f64()))
    }

    pub(super) fn builtin_time_time_ns(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("time_ns() expects no arguments"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RuntimeError::new("system time before epoch"))?;
        let nanos = now
            .as_nanos()
            .min(i64::MAX as u128)
            .try_into()
            .unwrap_or(i64::MAX);
        Ok(Value::Int(nanos))
    }

    pub(super) fn builtin_time_localtime(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_time_time_tuple(args, kwargs)
    }

    pub(super) fn builtin_time_gmtime(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        self.builtin_time_time_tuple(args, kwargs)
    }

    pub(super) fn builtin_time_time_tuple(
        &mut self,
        mut args: Vec<Value>,
        mut kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError::new(
                "time tuple function expects at most one argument",
            ));
        }
        let kw_secs = kwargs.remove("secs");
        if !kwargs.is_empty() {
            return Err(RuntimeError::new(
                "time tuple function got an unexpected keyword argument",
            ));
        }
        if kw_secs.is_some() && !args.is_empty() {
            return Err(RuntimeError::new(
                "time tuple function got multiple values for 'secs'",
            ));
        }
        let secs = if let Some(value) = kw_secs {
            if matches!(value, Value::None) {
                unix_seconds_now()
            } else {
                value_to_f64(value)?.trunc() as i64
            }
        } else if args.is_empty() || matches!(args[0], Value::None) {
            unix_seconds_now()
        } else {
            value_to_f64(args.remove(0))?.trunc() as i64
        };
        let parts = split_unix_timestamp(secs);
        Ok(self.heap.alloc_tuple(vec![
            Value::Int(parts.year as i64),
            Value::Int(parts.month as i64),
            Value::Int(parts.day as i64),
            Value::Int(parts.hour as i64),
            Value::Int(parts.minute as i64),
            Value::Int(parts.second as i64),
            Value::Int(parts.weekday as i64),
            Value::Int(parts.yearday as i64),
            Value::Int(parts.isdst as i64),
        ]))
    }

    pub(super) fn builtin_time_strftime(
        &mut self,
        mut args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.is_empty() || args.len() > 2 {
            return Err(RuntimeError::new(
                "strftime() expects format and optional tuple",
            ));
        }
        let format = match args.remove(0) {
            Value::Str(format) => format,
            _ => return Err(RuntimeError::new("strftime() format must be str")),
        };
        let parts = if args.is_empty() {
            split_unix_timestamp(unix_seconds_now())
        } else {
            time_parts_from_value(&args[0])?
        };
        Ok(Value::Str(format_strftime(&format, parts)))
    }

    pub(super) fn builtin_time_monotonic(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || !args.is_empty() {
            return Err(RuntimeError::new("monotonic() expects no arguments"));
        }
        let start = MONOTONIC_START.get_or_init(Instant::now);
        Ok(Value::Float(start.elapsed().as_secs_f64()))
    }

    pub(super) fn builtin_time_sleep(
        &mut self,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !kwargs.is_empty() || args.len() != 1 {
            return Err(RuntimeError::new("sleep() expects one argument"));
        }
        let seconds = value_to_f64(args[0].clone())?;
        if seconds < 0.0 {
            return Err(RuntimeError::new("sleep length must be non-negative"));
        }
        std::thread::sleep(Duration::from_secs_f64(seconds));
        Ok(Value::None)
    }
}
