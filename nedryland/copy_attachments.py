import argparse
import json
import os
import os.path
import shutil
import sys


def copyFileToOutput(path: str, target_filename: str) -> None:
    os.makedirs(os.path.dirname(target_filename), exist_ok=True)
    shutil.copy(path, target_filename)


def read_infile(path: str) -> dict:
    with open(args.infile) as json_file:
        return json.load(json_file)


if __name__ == "__main__":

    parser = argparse.ArgumentParser(description="Copy attachments to a folder")
    parser.add_argument("outputfolder", help="Folder to put attachments in")
    parser.add_argument("infile", help="Input json file")
    parser.add_argument("outfile", help="Output json file")

    parser.add_argument("--quiet", action="store_true", default=False)

    args = parser.parse_args()

    json_data = {}
    try:
        json_data = read_infile(args.infile)
    except Exception as e:
        print(f"Failed to read JSON data from {args.infile}: {e}")
        sys.exit(1)

    json_data = json_data["manifest"]
    output_data = json_data.copy()

    # only keep attachments that are either required or exists
    required_or_existing_attachments = {
        n: a
        for n, a in json_data.get("attachments").items()
        if a.get("required", True) or os.path.exists(a["path"])
    }

    for name, attachment in required_or_existing_attachments.items():
        target = os.path.join(args.outputfolder, attachment["path"])
        if not os.path.exists(attachment["path"]):
            print(
                f"Attachment {name} is required but does not exist at {attachment['path']}, exiting..."
            )
            sys.exit(1)

        if not args.quiet:
            print(f"Copying attachment {name} at {attachment['path']} to {target}")

        try:
            copyFileToOutput(attachment["path"], target)
        except Exception as e:
            print(
                "Failed to copy attachment {name} at {attachment['path']} to {target}: {e}"
            )
            sys.exit(1)

    output_data["attachments"] = required_or_existing_attachments
    output_data = {"manifest": output_data}

    with open(args.outfile, "w") as outfile:
        json.dump(output_data, outfile)

    print(f"JSON written to {args.outfile}")
