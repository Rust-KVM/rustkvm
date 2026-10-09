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
# Anonymous share-link access to the consumer OneDrive API needs a "Badger"
# guest token; this is the app id the OneDrive web viewer requests it with.
BADGER_TOKEN_URL = "https://api-badgerp.svc.ms/v1.0/token"
BADGER_APP_ID = "5cbed6ac-a083-4e14-b191-b4ba07653de2"


def badger_token():
    body = json.dumps({"appId": BADGER_APP_ID}).encode()
    req = urllib.request.Request(
        BADGER_TOKEN_URL, data=body, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.load(resp)["token"]


def onedrive_get(url, _auth=[]):
    if not _auth:
        _auth.append(f"Badger {badger_token()}")
    req = urllib.request.Request(
        url,
        headers={
            "Accept": "application/json",
            "Authorization": _auth[0],
            "Prefer": "autoredeem",
        },
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.load(resp)


def onedrive_children(url):
    url += "?$top=200"
    while url:
        page = onedrive_get(url)
        yield from page.get("value", [])
        url = page.get("@odata.nextLink")


def onedrive_find(share_url, name, max_depth=4):
    share_id = "u!" + base64.urlsafe_b64encode(share_url.encode()).decode().rstrip("=")
    queue = [(f"{ONEDRIVE_API}/shares/{share_id}/driveItem/children", 0, "")]
    while queue:
        children_url, depth, path = queue.pop(0)
        for child in onedrive_children(children_url):
            child_path = f"{path}/{child['name']}"
            print(child_path, flush=True)
            if "folder" in child and depth < max_depth:
                drive = child["parentReference"]["driveId"]
                url = f"{ONEDRIVE_API}/drives/{drive}/items/{child['id']}/children"
                queue.append((url, depth + 1, child_path))
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
