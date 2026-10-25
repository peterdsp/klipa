#!/usr/bin/env python3
"""Create the Mac App Store version for an uploaded build and submit it.

Uses the SAME App Store Connect API key the release workflow already holds in
CI secrets, so it needs no new Apple access. Every mutation is skipped unless
ASC_SUBMIT=1, so the default run only reports what it would do.

The steps, in order, each one skipped when it is already satisfied:
  1. find the app and the newest VALID build with the target version
  2. find or create the editable App Store version
  3. write the "What's New" text for en-US
  4. attach the build to that version
  5. create a review submission, add the version to it, and submit it

Env:
  ASC_KEY_ID, ASC_ISSUER_ID, ASC_API_KEY_P8_BASE64  (from CI secrets)
  ASC_BUNDLE_ID     default dev.peterdsp.klipa
  ASC_VERSION       marketing version to ship, e.g. 0.6.3
  ASC_BUILD_VERSION build (CFBundleVersion) to attach; default ASC_VERSION
  ASC_WHATS_NEW     release-notes text for the store listing
  ASC_SUBMIT        "1" to actually write; anything else is a dry run
"""
import base64
import json
import os
import sys
import time
import urllib.error
import urllib.request

import jwt  # PyJWT + cryptography

API = "https://api.appstoreconnect.apple.com"
PLATFORM = "MAC_OS"
LOCALE = "en-US"
# States where the version is still ours to edit rather than Apple's to review.
EDITABLE = {
    "PREPARE_FOR_SUBMISSION",
    "DEVELOPER_REJECTED",
    "REJECTED",
    "METADATA_REJECTED",
    "INVALID_BINARY",
}

DRY = os.environ.get("ASC_SUBMIT") != "1"


def token():
    key_id = os.environ["ASC_KEY_ID"]
    issuer = os.environ["ASC_ISSUER_ID"]
    p8 = base64.b64decode(os.environ["ASC_API_KEY_P8_BASE64"]).decode()
    now = int(time.time())
    return jwt.encode(
        {"iss": issuer, "iat": now, "exp": now + 15 * 60, "aud": "appstoreconnect-v1"},
        p8,
        algorithm="ES256",
        headers={"kid": key_id, "typ": "JWT"},
    )


def call(method, path, tok, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(API + path, data=data, method=method)
    req.add_header("Authorization", f"Bearer {tok}")
    if data:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            raw = r.read()
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        print(f"HTTP {e.code} {method} {path}\n{e.read().decode()[:900]}", file=sys.stderr)
        raise


def write(method, path, tok, body, what):
    """Perform a mutation, or describe it when this is a dry run."""
    if DRY:
        print(f"  would {what}")
        return None
    print(f"  {what}")
    return call(method, path, tok, body)


def main():
    bundle = os.environ.get("ASC_BUNDLE_ID", "dev.peterdsp.klipa")
    version = os.environ["ASC_VERSION"]
    build_version = os.environ.get("ASC_BUILD_VERSION") or version
    whats_new = os.environ.get("ASC_WHATS_NEW", "").strip()
    tok = token()

    print(f"mode: {'DRY RUN (no writes)' if DRY else 'SUBMITTING'}")

    apps = call("GET", f"/v1/apps?filter[bundleId]={bundle}&limit=1", tok)["data"]
    if not apps:
        print(f"No app in App Store Connect for bundle id {bundle}.")
        return 1
    app_id = apps[0]["id"]
    live = apps[0]["attributes"].get("name")
    print(f"app: {live} ({bundle}) id={app_id}")

    # 1. the build to ship
    builds = call(
        "GET",
        f"/v1/builds?filter[app]={app_id}&filter[version]={build_version}"
        f"&limit=10&sort=-uploadedDate"
        f"&fields[builds]=version,processingState,uploadedDate,expired",
        tok,
    )["data"]
    usable = [
        b
        for b in builds
        if b["attributes"]["processingState"] == "VALID" and not b["attributes"].get("expired")
    ]
    if not usable:
        print(f"No VALID, unexpired build {build_version}. Nothing to submit.")
        return 1
    build = usable[0]
    print(f"build: {build_version} id={build['id']} uploaded={build['attributes']['uploadedDate']}")

    # 2. the App Store version
    versions = call(
        "GET",
        f"/v1/appStoreVersions?filter[app]={app_id}&filter[platform]={PLATFORM}"
        f"&limit=10&fields[appStoreVersions]=versionString,appStoreState,createdDate",
        tok,
    )["data"]
    for v in versions[:5]:
        a = v["attributes"]
        print(f"  existing version {a['versionString']:<8} {a['appStoreState']}")

    target = next((v for v in versions if v["attributes"]["versionString"] == version), None)
    if target and target["attributes"]["appStoreState"] not in EDITABLE:
        print(
            f"Version {version} exists in state {target['attributes']['appStoreState']}, "
            "which is not editable. Nothing to do."
        )
        return 0

    if target:
        ver_id = target["id"]
        print(f"version {version}: reusing {ver_id} ({target['attributes']['appStoreState']})")
    else:
        body = {
            "data": {
                "type": "appStoreVersions",
                "attributes": {
                    "platform": PLATFORM,
                    "versionString": version,
                    "releaseType": "AFTER_APPROVAL",
                },
                "relationships": {"app": {"data": {"type": "apps", "id": app_id}}},
            }
        }
        created = write("POST", "/v1/appStoreVersions", tok, body, f"create version {version}")
        if created is None:
            print("\nDry run complete. Re-run with submit enabled to apply.")
            return 0
        ver_id = created["data"]["id"]
        print(f"  created {ver_id}")

    # 3. What's New
    if whats_new:
        locs = call(
            "GET",
            f"/v1/appStoreVersions/{ver_id}/appStoreVersionLocalizations"
            f"?limit=50&fields[appStoreVersionLocalizations]=locale",
            tok,
        )["data"]
        loc = next((l for l in locs if l["attributes"]["locale"] == LOCALE), None)
        if loc:
            write(
                "PATCH",
                f"/v1/appStoreVersionLocalizations/{loc['id']}",
                tok,
                {
                    "data": {
                        "type": "appStoreVersionLocalizations",
                        "id": loc["id"],
                        "attributes": {"whatsNew": whats_new},
                    }
                },
                f"set What's New for {LOCALE}",
            )
        else:
            print(f"  no {LOCALE} localization on this version; leaving What's New alone")

    # 4. attach the build
    write(
        "PATCH",
        f"/v1/appStoreVersions/{ver_id}/relationships/build",
        tok,
        {"data": {"type": "builds", "id": build["id"]}},
        f"attach build {build_version}",
    )

    # 5. review submission
    subs = call(
        "GET",
        f"/v1/reviewSubmissions?filter[app]={app_id}&filter[platform]={PLATFORM}"
        f"&filter[state]=READY_FOR_REVIEW,WAITING_FOR_REVIEW,IN_REVIEW,UNRESOLVED_ISSUES"
        f"&limit=10&fields[reviewSubmissions]=state,submitted",
        tok,
    )["data"]
    open_sub = next((s for s in subs if not s["attributes"].get("submitted")), None)
    if subs and open_sub is None:
        print(f"  a review submission is already in flight ({subs[0]['attributes']['state']})")
        return 0

    if open_sub:
        sub_id = open_sub["id"]
        print(f"  reusing open review submission {sub_id}")
    else:
        created = write(
            "POST",
            "/v1/reviewSubmissions",
            tok,
            {
                "data": {
                    "type": "reviewSubmissions",
                    "attributes": {"platform": PLATFORM},
                    "relationships": {"app": {"data": {"type": "apps", "id": app_id}}},
                }
            },
            "create review submission",
        )
        if created is None:
            print("\nDry run complete. Re-run with submit enabled to apply.")
            return 0
        sub_id = created["data"]["id"]

    write(
        "POST",
        "/v1/reviewSubmissionItems",
        tok,
        {
            "data": {
                "type": "reviewSubmissionItems",
                "relationships": {
                    "reviewSubmission": {"data": {"type": "reviewSubmissions", "id": sub_id}},
                    "appStoreVersion": {"data": {"type": "appStoreVersions", "id": ver_id}},
                },
            }
        },
        f"add version {version} to the submission",
    )
    write(
        "PATCH",
        f"/v1/reviewSubmissions/{sub_id}",
        tok,
        {"data": {"type": "reviewSubmissions", "id": sub_id, "attributes": {"submitted": True}}},
        "submit for App Review",
    )

    print("\nDry run complete. Re-run with submit enabled to apply." if DRY else "\nSubmitted.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
