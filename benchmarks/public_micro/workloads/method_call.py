class Counter:
    def __init__(self):
        self.value = 0

    def bump(self):
        self.value += 1
        return self.value


def main():
    counter = Counter()
    iterations = 1_500_000
    for _ in range(iterations):
        counter.bump()
    print(counter.value)


if __name__ == "__main__":
    main()
