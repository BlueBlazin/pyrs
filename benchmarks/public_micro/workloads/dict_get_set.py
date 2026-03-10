def main():
    size = 200_000
    data = {}
    for value in range(size):
        data[value] = value
    total = 0
    for value in range(size):
        total += data[value]
    print(total)


if __name__ == "__main__":
    main()
