class Payload:
    def __init__(self, value):
        self.value = value


def main():
    payload = Payload(7)
    total = 0
    iterations = 5_000_000
    for _ in range(iterations):
        total += payload.value
    print(total)


if __name__ == "__main__":
    main()
