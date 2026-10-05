#!/usr/bin/env python3
"""Report the real App Store Connect state of the uploaded MAS build.

Read-only. Uses the SAME App Store Connect API key the release workflow already
holds in CI secrets (ASC_KEY_ID, ASC_ISSUER_ID, ASC_API_KEY_P8_BASE64); it
needs no new Apple access. An "UPLOAD SUCCEEDED" receipt is not TestFlight
availability, so this answers: did processing finish, is export compliance
answered, and is the build actually usable for testing yet.

Env:
  ASC_KEY_ID, ASC_ISSUER_ID, ASC_API_KEY_P8_BASE64  (from CI secrets)
  ASC_BUNDLE_ID   default dev.peterdsp.klipa
"""
import base64
import json
import os
import sys
import time
import urllib.request
import urllib.error

import jwt  # PyJWT + cryptography

API = "https://api.appstoreconnect.apple.com"


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


def get(path, tok):
    req = urllib.request.Request(API + path, headers={"Authorization": f"Bearer {tok}"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        print(f"HTTP {e.code} for {path}: {e.read().decode()[:500]}", file=sys.stderr)
        raise


def main():
    bundle = os.environ.get("ASC_BUNDLE_ID", "dev.peterdsp.klipa")
    tok = token()

    apps = get(f"/v1/apps?filter[bundleId]={bundle}&limit=1", tok)["data"]
    if not apps:
        print(f"No app in App Store Connect for bundle id {bundle}.")
        return 0
    app_id = apps[0]["id"]
    print(f"app: {apps[0]['attributes'].get('name')} ({bundle}) id={app_id}\n")

    builds = get(
        f"/v1/builds?filter[app]={app_id}&limit=8&sort=-uploadedDate"
        f"&include=preReleaseVersion"
        f"&fields[builds]=version,processingState,uploadedDate,expired,"
        f"usesNonExemptEncryption,buildAudienceType",
        tok,
    )
    data = builds.get("data", [])
    pre = {i["id"]: i for i in builds.get("included", []) if i["type"] == "preReleaseVersions"}
    if not data:
        print("No builds found yet. ASC can take a few minutes after UPLOAD "
              "SUCCEEDED to register the build; re-run shortly.")
        return 0

    print(f"{'buildVer':>9}  {'short':>6}  {'processing':>11}  {'encryption':>12}  uploaded")
    for b in data:
        a = b["attributes"]
        rel = b.get("relationships", {}).get("preReleaseVersion", {}).get("data")
        short = pre.get(rel["id"], {}).get("attributes", {}).get("version") if rel else "?"
        enc = a.get("usesNonExemptEncryption")
        enc_s = "MISSING" if enc is None else ("exempt" if enc is False else "non-exempt")
        print(f"{a.get('version'):>9}  {str(short):>6}  {str(a.get('processingState')):>11}"
              f"  {enc_s:>12}  {a.get('uploadedDate')}")

    # Focus on our candidate: build 0.6.3.
    cand = next((b for b in data if b["attributes"].get("version") == os.environ.get("ASC_EXPECT_BUILD", "0.6.3")), None)
    print()
    if not cand:
        print(f"Candidate build {os.environ.get('ASC_EXPECT_BUILD','0.6.3')} not listed yet "
              "(still registering or processing). Re-run shortly.")
        return 0

    a = cand["attributes"]
    cid = cand["id"]
    state = a.get("processingState")
    enc = a.get("usesNonExemptEncryption")
    print(f"=== candidate build {a.get('version')} (id {cid}) ===")
    print(f"processingState: {state}")
    print(f"usesNonExemptEncryption: {enc}  "
          + ("(export compliance NOT answered -> blocks TestFlight until set)"
             if enc is None else "(export compliance answered)"))

    # Beta review + internal/external testing readiness.
    try:
        br = get(f"/v1/builds/{cid}/betaAppReviewSubmission", tok)
        bs = br.get("data")
        print("betaAppReviewSubmission: "
              + (bs["attributes"].get("betaReviewState") if bs else "none "
                 "(internal testing needs no beta review; external testing does)"))
    except Exception:
        print("betaAppReviewSubmission: none (external testing would require it)")

    print()
    actionable = []
    if state == "PROCESSING":
        actionable.append("Wait: ASC is still PROCESSING the build; it is not testable until VALID.")
    elif state == "FAILED" or state == "INVALID":
        actionable.append(f"Processing {state}: open the build in ASC for the exact error.")
    if enc is None:
        actionable.append("Answer export compliance (Info.plist ITSAppUsesNonExemptEncryption=false, "
                           "or set it in ASC) so the build becomes usable for TestFlight.")
    if not actionable:
        print("No blocking processing errors detected for the candidate.")
        print("For INTERNAL TestFlight testing: assign the build to an internal "
              "tester group (owner action in ASC); no beta review required.")
        print("For EXTERNAL testing: a Beta App Review is required first.")
    else:
        print("ACTIONABLE:")
        for x in actionable:
            print(" - " + x)
    return 0


if __name__ == "__main__":
    sys.exit(main())
