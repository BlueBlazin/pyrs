def main():
    total = 0
    iterations = 3_000_000
    for value in range(iterations):
        total += value
    print(total)


if __name__ == "__main__":
    main()
