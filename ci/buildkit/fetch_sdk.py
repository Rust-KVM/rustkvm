"""Download the FriendlyELEC Buildroot SDK tar from a Google Drive file or a
OneDrive shared folder (FriendlyELEC's download page mirrors both).

Usage: fetch_sdk.py <source> <file-name> <output>
"""

import base64
import json
import subprocess
import sys
import urllib.request

ONEDRIVE_API = "https://api.onedrive.com/v1.0"


def onedrive_get(url):
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.load(resp)


def onedrive_children(item):
    drive = item["parentReference"]["driveId"]
    url = f"{ONEDRIVE_API}/drives/{drive}/items/{item['id']}/children?$top=200"
    while url:
        page = onedrive_get(url)
        yield from page.get("value", [])
        url = page.get("@odata.nextLink")


def onedrive_find(share_url, name, max_depth=4):
    share_id = "u!" + base64.urlsafe_b64encode(share_url.encode()).decode().rstrip("=")
    root = onedrive_get(f"{ONEDRIVE_API}/shares/{share_id}/driveItem")
    queue = [(root, 0, root.get("name", ""))]
    while queue:
        folder, depth, path = queue.pop(0)
        for child in onedrive_children(folder):
            child_path = f"{path}/{child['name']}"
            print(child_path, flush=True)
            if "folder" in child and depth < max_depth:
                queue.append((child, depth + 1, child_path))
            elif child["name"] == name:
                return child
    sys.exit(f"{name} not found in {share_url}")


def main():
    source, name, output = sys.argv[1:4]
    if "1drv.ms" in source or "onedrive.live.com" in source:
        item = onedrive_find(source, name)
        print(f"found {item['name']} size={item['size']}", flush=True)
        subprocess.run(
            ["curl", "-fL", "--retry", "5", "--retry-all-errors", "-C", "-",
             "-o", output, item["@content.downloadUrl"]],
            check=True,
        )
    else:
        subprocess.run(
            ["pipx", "run", "gdown==5.2.0", "--fuzzy", source, "-O", output],
            check=True,
        )


if __name__ == "__main__":
    main()
