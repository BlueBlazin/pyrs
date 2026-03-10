import json


PAYLOAD = '{"a":1,"b":[1,2,3],"c":"p"}'


def main():
    encoded_total = 0
    decoded_total = 0
    iterations = 100_000
    for _ in range(iterations):
        obj = json.loads(PAYLOAD)
        encoded = json.dumps(obj, sort_keys=True, separators=(",", ":"))
        encoded_total += len(encoded)
        decoded_total += obj["a"] + obj["b"][0]
    print(f"{encoded_total}:{decoded_total}")


if __name__ == "__main__":
    main()
