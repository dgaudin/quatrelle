# OpenPGP: read what comes, then sign and encrypt, inactive until a key is made

Status: lot P5 of the upstream ZIDAN44 v0.32 survey (`~/mailcraft/merge-tools/survey-v0.32.md`,
« Décisions » n° 7), the owner's own request of 2026-11-01 (carnet item `openpgp`). Designed with the
owner on 2026-11-05, approach A: upstream's OpenPGP (on rPGP: `576b2770`, `896db24a`, `f32b54b0`,
`775f386b`, `7e743c43`) taken into the fork's own S/MIME layout, plus key creation, which upstream
lacks. The owner wants the feature present but **not active**: nothing changes in what is sent until
a key exists for an account.

## Why

- Quatrelle reads and sends S/MIME only. A message signed or encrypted with OpenPGP (Thunderbird,
  Proton, GnuPG users) shows as an unexplained attachment (`signature.asc`, `encrypted.asc`).
- The owner has no OpenPGP key yet. To receive encrypted mail and answer in kind, he needs a key
  pair made where he reads his mail, with its public part reaching his correspondents.

## What the owner said (2026-11-05)

1. First use: read what comes, see whether a signature is good, answer in kind.
2. No key yet: **Quatrelle makes the key**; its secret part goes to the system keychain.
3. **Not active now**: he turns it on later, account by account.
4. Approach A (upstream's OpenPGP, adapted), not read-only first, not GnuPG.
5. Agreed in the design: reading works without a key; correspondents' keys are learnt locally only (no
   key servers: the project connects only to the user's own servers); S/MIME is unchanged; a changed
   key for an address is said once in the reading pane.

## Scope

In: reading (verify, decrypt), the account's own keys (create, extend, back up, import, export, forget),
correspondents' keys (Autocrypt, attached keys, a file), sending (sign, encrypt, Autocrypt), the choice
between S/MIME and OpenPGP, the encryption line naming recipients without a key.

Out: key servers and WKD (they are other people's servers), the web of trust and key signing,
OpenPGP on smart cards, the Autocrypt Setup Message, publishing a revocation, OpenPGP in the browser
build (it answers OpenPGP jobs with "not available here yet", as upstream).

## Architecture

The pieces follow S/MIME's place in the fork (`crates/smime`, `mime/src/smime.rs`,
`net/src/worker/smime.rs`, `engine/src/cmd/security.rs`, `ui-egui/src/panes/backstage/security.rs`,
`ui-egui/src/reading/security.rs`).

1. **`quatrelle-pgp`** (new crate, L1, portable, wasm-safe), from `576b2770`, on rPGP 0.21 (MIT or
   Apache-2.0, the RustCrypto generation the S/MIME crate uses). Keys read and checked (user IDs,
   subkeys, expiry and revocation from their self-signatures, each checked; a subkey or user ID bound
   by another key doesn't count); detached, cleartext and inline signatures verified against the keys
   that could have made them; encryption (AES-256, SEIPDv1) to each key's newest valid encryption
   subkey; decryption with what it decompresses to bounded; Autocrypt headers read and written; inline
   PGP blocks found. It never reads the clock (the engine passes the time). rPGP runs behind a guard: a
   panic in it is an error. **Added here**: making a key pair (below).
2. **`mime`: PGP/MIME**, from `896db24a`, beside `smime.rs`: `multipart/signed` with
   `application/pgp-signature`, `multipart/encrypted` with `application/pgp-encrypted` (RFC 3156), and a
   PGP block in a one-text message, each as a whole message only; written signed, then encrypted
   (RFC 3156 §6.1), with the header fields inside (RFC 9788). Autocrypt headers and
   `application/pgp-keys` parts read. The list shows PGP/MIME mail as signed or encrypted with the
   existing markers (`Smime::Signed`, `Smime::Encrypted`, renamed only if the plan finds it clearer).
3. **`net/src/worker/pgp.rs`**, after `worker/smime.rs`: secret keys only in the system keychain
   (through the pool's credentials, as S/MIME's PKCS #8), decryption and signing there; decrypted
   text kept in memory only.
4. **Engine**: `pgp.*` commands (create, extend, export public, back up, import, forget, list,
   people's keys); `compose.security` gains a scheme (`auto`, `smime`, `pgp`); `compose.encryption`
   (who can't receive it encrypted, issue #6 upstream). The commands follow the fork's rules: they
   never open dialogs, and `every_command_survives_junk_params` covers them.
5. **Interface**: an OpenPGP card under the S/MIME card in File › Info › Security; the reading pane's
   security bar; the scheme choice in the message window when the account has both.

## The account's own key

- **Nothing exists until it's made.** The OpenPGP card says what OpenPGP is for and offers
  « Create an OpenPGP key » (and « Import a key… »).
- **Made by Quatrelle**: an OpenPGP v4 key, an Ed25519 primary key (certify, sign) and an X25519
  encryption subkey, the form GnuPG 2.2+, Thunderbird and Proton read. One user ID: the account's
  display name and address. Expiry **3 years** on both, extended by « Extend » (re-signed for 3 years
  from that day); the card says so a month before it runs out.
- **The secret part** goes to the system keychain, without a passphrase for daily use (as the S/MIME
  key imported from a `.p12`). Never in Quatrelle's files or log (`Secret`'s `Debug` is redacted).
- **Backup**: right after making it, Quatrelle offers « Save a backup… »: the secret key armored in a
  `.asc` file, protected by a passphrase the user types twice, with an S2K GnuPG 2.2 reads (iterated
  and salted SHA-256; not Argon2, which older GnuPG can't read). It can be skipped; the card then keeps
  saying that no backup was made, until one is.
- **Export** of the public key to a file; « Attach my public key » in the message window.
- **Import** of a secret key file with its passphrase (upstream's `pgp.import`).
- **Older keys** stay, to read older mail encrypted to them (as S/MIME's older certificates), each
  with « Forget », which says what can no longer be read.
- **Keychain unavailable**: the card says why and nothing is made or turned on.

## Correspondents' keys

- Learnt **locally only**, from their mail:
  - an Autocrypt header, for the From address only;
  - an attached key (`application/pgp-keys`), for the From address only;
  - a message's key replaces the one an older message gave (by the message's date), **never one
    imported by hand**.
- Or imported from a file.
- Kept in a store document of the account (hostile input: a damaged one is set aside, never fatal),
  at most 10,000 people's keys, the one used least recently let go first, so a flood of mail can't
  grow it without end; a key imported by hand is let go only by « Forget ».
- Listed in the OpenPGP card: address, fingerprint, how it was learnt (« from their mail » or
  « imported by you »), « Forget ».
- **A key that changes** for an address (a new one learnt from mail where another was known) is said
  once in that message's security bar: « This person's key changed ». The old fingerprint is kept with
  the new one until then.

## Reading (always on, with or without a key of one's own)

- **Signed** (PGP/MIME or a signed block in a plain text): the security bar says « Signed by Name
  <address> », and one of: the signature is good; it is wrong; the key isn't known, so it can't be
  checked; it was signed for another address than the sender's. It also says how the key was learnt,
  and a key change once.
- **Encrypted to the user**: decrypted in memory by the pool; the bar says « Encrypted ». The decrypted
  text is never written to the cache, so the search finds such mail by its sender and its subject in
  the clear, not by its text (the same caution as S/MIME). A protected subject (RFC 9788) is shown once
  decrypted.
- **Encrypted without the key**: « This message is encrypted to a key Quatrelle doesn't have ».
- Wording: Quatrelle's own, in the style of the S/MIME bar's strings, in the 13 catalogs.

## Sending (nothing until the account has a key)

- The message window's existing Sign and Encrypt serve OpenPGP too. With both an S/MIME certificate
  and an OpenPGP key, a choice appears: **Automatic** (the default: the scheme every recipient can
  receive; a reply, the scheme of the message it answers), **S/MIME**, **OpenPGP**.
- **Encrypt** encrypts to every recipient's key and to the sender's own (the sent copy stays
  readable). A recipient with no known key is named in the window, and encryption can't be on. An
  option, off unless chosen, turns encryption on by itself when every recipient can receive it.
- **Autocrypt**: once the key exists, every message from that account carries the public key in an
  Autocrypt header (no `prefer-encrypt`), unless « Send my public key with my messages » (on when the
  key is made) is turned off.

### Exchange: to be checked on the owner's server before turning it on there

Quatrelle sends over EWS the whole message (`CreateItem` with `MimeContent`), and Exchange turns it into
its own form. It handles S/MIME signatures; nothing says it keeps an OpenPGP signature byte for byte,
sending or receiving. The day the owner turns OpenPGP on for his Exchange account, a signed message
from it to his Gmail must verify in Quatrelle, and one signed from Gmail must verify when read through
Exchange. If either fails, Exchange needs another form of sending; that is a later piece of work, not
this one.

## Robustness

- Every key, signature, message and Autocrypt header is hostile: caps on sizes, nesting and what
  decryption decompresses to; no `unwrap`, `expect` or panic; rPGP behind its guard.
- Fuzz targets: upstream's `pgp` (keys and messages) and its PGP/MIME one, with their seeds and
  dictionary.
- The test files upstream made with GnuPG 2.4 stay byte for byte (`.gitattributes`, `775f386b`).

## Testing

- Upstream's GnuPG test files (keys, signed and encrypted mail) and its hostile-input tests.
- A key made by Quatrelle read back; sign → verify and encrypt → decrypt round trips (a proptest).
- An ignored test, run by hand where GnuPG is installed: GnuPG reads a key and a message Quatrelle made,
  and Quatrelle reads GnuPG's.
- The fake IMAP and EWS servers: a signed and an encrypted message through sync and through sending.
- Headless frames of the OpenPGP card, the security bar and the message window's choice, looked at.
- `every_command_survives_junk_params` with the `pgp.*` commands; `cargo deny` for the new crates
  (about 50, RustCrypto); `cargo xtask ci`, wasm included.

## Delivery: four lots, one agent each, in order

1. **`quatrelle-pgp`**: the crate from `576b2770`, key creation, extension and the backup's S2K, its
   fuzz target, `cargo deny`.
2. **PGP/MIME in `mime`**: `896db24a`, the list's markers, its fuzz target.
3. **Keys and reading**: the pool's keychain, the `pgp.*` commands, the OpenPGP card (create, back up,
   import, export, extend, forget, people's keys), learning keys from mail, the changed-key notice, the
   security bar, decryption in memory (`f32b54b0`, its reading half).
4. **Sending**: the scheme choice, sign and encrypt, Autocrypt, the encryption line and its option,
   the docs (`f32b54b0`, its sending half; `775f386b`, `7e743c43`).

Each lot builds and passes `cargo xtask ci` on its own; nothing is active until a key is made, so
each can go into a development build without changing anything for the owner.
