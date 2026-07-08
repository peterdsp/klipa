# klipa license server (Ko-fi)

klipa's paid unlock (direct-download builds only) is issued by a
self-hosted license server, **not** by a klipa-specific box. Ko-fi allows
only one webhook URL per account, and that URL already points at the
shared multi-product server that also serves PromptBar:

- Source of truth: **`PromptBar/scripts/pi-license-server/app.py`**
- Deployed on the Pi at `licenses.peterdsp.dev` (Cloudflare Tunnel ->
  gunicorn on `:8000`), systemd unit `promptbar-licenses.service`.

The server is product-aware: each Ko-fi shop order is matched to a
product, signed with that product's own Ed25519 key, archived, and
emailed. A license signed for one product cannot unlock another.

## How klipa activation works

```
Ko-fi sale ─► /webhook ─► match klipa item ─► sign Ed25519 license
                                              ─► archive <email>.klipa
                                              ─► email the buyer
klipa app  ─► POST /activate {email, product:"klipa"}
           ◄─ signed license blob
           ─► verify signature offline against LICENSE_PUBKEY_B64
```

The buyer never handles a key: they copy the email they used on Ko-fi,
click **Activate**, and klipa fetches + verifies the license.

## The keypair

- **Public key** (embedded in the app, safe to commit) - the
  `LICENSE_PUBKEY_B64` constant in
  [`../../crates/klipa-ui/src/license.rs`](../../crates/klipa-ui/src/license.rs):

  ```
  jbSjJelSCv+gs0bXuaVnsKsu/IyhGGUrjJ+ProKrLPo=
  ```

- **Private key** - lives only on the Pi
  (`/home/peterdsp/promptbar/klipa-license-private.key`, raw 32-byte
  Ed25519, base64) and in the offline signing backup. Never in the repo.

Generate a fresh pair with:

```bash
python3 - <<'PY'
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
from base64 import b64encode
p = Ed25519PrivateKey.generate()
priv = p.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw, serialization.NoEncryption())
pub  = p.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
print("PRIVATE:", b64encode(priv).decode())
print("PUBLIC :", b64encode(pub).decode())
PY
```

Put `PUBLIC` in `license.rs`; copy `PRIVATE` to the Pi and point
`KLIPA_PRIVATE_KEY` at it.

## klipa's slice of the Pi `.env`

Added alongside the existing PromptBar config in
`/home/peterdsp/promptbar/.env`:

```ini
KLIPA_PRIVATE_KEY=/home/peterdsp/promptbar/klipa-license-private.key
KLIPA_MIN_VERSION=0.4.0
KLIPA_LINK_CODES=4e1cf2ac40      # klipa's Ko-fi direct_link_code
KLIPA_NAME_MATCH=klipa           # fallback item-name match
```

After changing the key/env, restart the service:

```bash
ssh peterdsp@192.168.10.10 'sudo systemctl restart promptbar-licenses'
curl -s https://licenses.peterdsp.dev/health   # both products should list
```

## Verifying end to end

```bash
# klipa admin/license for a known email (the operator's admin license is
# auto-bootstrapped at server startup):
curl -s -X POST https://licenses.peterdsp.dev/activate \
  -H 'Content-Type: application/json' \
  -d '{"email":"info@peterdsp.dev","product":"klipa"}'
```

A successful response is the signed license JSON (`product: "klipa"`,
plus a `signature`). In the app: copy the email, click **Activate**.
