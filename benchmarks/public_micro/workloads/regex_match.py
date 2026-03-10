import re


def main():
    pattern = re.compile(r"^[a-z]+_[0-9]+$")
    text = "pyrs_314159"
    matches = 0
    iterations = 500_000
    for _ in range(iterations):
        if pattern.match(text):
            matches += 1
    print(matches)


if __name__ == "__main__":
    main()
