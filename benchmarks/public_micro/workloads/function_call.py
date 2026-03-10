def bump(value):
    return value + 1


def main():
    value = 0
    iterations = 2_000_000
    for _ in range(iterations):
        value = bump(value)
    print(value)


if __name__ == "__main__":
    main()
