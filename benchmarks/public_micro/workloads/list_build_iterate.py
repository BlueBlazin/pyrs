def main():
    size = 400_000
    data = []
    for value in range(size):
        data.append(value)
    total = 0
    for value in data:
        total += value
    print(total)


if __name__ == "__main__":
    main()
