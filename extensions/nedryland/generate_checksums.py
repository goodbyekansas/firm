import argparse
import hashlib
import json
import os.path
import sys


def generateSha256Checksum(attachment: dict) -> str:
    sha256 = hashlib.sha256()
    if os.path.exists(attachment["path"]):
        with open(attachment["path"], "rb") as f:
            for byte_block in iter(lambda: f.read(4096), b""):
                sha256.update(byte_block)

    return sha256.hexdigest()


def generateSha512Checksum(attachment: dict) -> str:
    sha512 = hashlib.sha512()
    if os.path.exists(attachment["path"]):
        with open(attachment["path"], "rb") as f:
            for byte_block in iter(lambda: f.read(4096), b""):
                sha512.update(byte_block)

    return sha512.hexdigest()


def generateChecksums(data: dict, sha512: bool) -> dict:
    dataWithChecksums = data.copy()
    for name, attachment in data.items():
        dataWithChecksums[name].update(
            {"checksums": {"sha256": generateSha256Checksum(attachment)}}
        )

        if sha512:
            dataWithChecksums[name]["checksums"]["sha512"] = generateSha512Checksum(
                attachment
            )

    return dataWithChecksums


def read_infile(path: str) -> dict:
    with open(args.infile) as json_file:
        return json.load(json_file)


if __name__ == "__main__":

    parser = argparse.ArgumentParser(description="Generate checksums for attachments")
    parser.add_argument("infile", help="Input json file")
    parser.add_argument("outfile", help="Output json file")
    parser.add_argument("--sha512", action="store_true", default=False)
    parser.add_argument(
        "--code-root",
        help="Path to the folder where the deployed code is relative to.",
        default=os.environ.get("out", ""),
    )

    args = parser.parse_args()

    json_data = {}
    try:
        json_data = read_infile(args.infile)
    except Exception as e:
        print(f"Failed to read JSON data from {args.infile}: {e}")
        sys.exit(1)

    json_data = json_data["manifest"]
    output = json_data.copy()
    output["attachments"] = generateChecksums(
        json_data.get("attachments", {}), args.sha512
    )
    output["code"] = generateChecksums(
        {"code": {"path": os.path.join(args.code_root, json_data["code"]["path"])}},
        args.sha512,
    )["code"]

    output = {"manifest": output}

    with open(args.outfile, "w") as outfile:
        json.dump(output, outfile)

    print(f"JSON written to {args.outfile}")
