# Hosting Quatrelle for the web

`quatrelle-web-<version>.zip` (from the GitHub release, or `packaging/web/package.sh`) holds a
static site in `quatrelle-web-<version>/`:

| File | What it is |
|---|---|
| `index.html` | The page. It loads everything through relative URLs. |
| `quatrelle-web-<hash>.js` | wasm-bindgen glue (generated, ES module) |
| `quatrelle-web-<hash>_bg.wasm` | The app |
| `_headers`, `.htaccess` | Sample header rules for Netlify/Cloudflare Pages and Apache |

There is no server-side code. Upload the folder's contents anywhere that serves static files.

## What works in the browser

A web page can't open IMAP or SMTP connections, so the browser version syncs **JMAP** accounts
(RFC 8620/8621: Fastmail, Stalwart, Cyrus, Apache James…) over `fetch`, with the same code as the
desktop app. Everything else in the window works as on the desktop: the list, conversations,
reading, writing with attachments, folders, rules, categories, search, the calendar and people
kept with the account, and every language.

- **The mail server must allow the page.** The browser only lets the page talk to a JMAP server
  that answers with CORS headers for the page's origin (`Access-Control-Allow-Origin`, and
  `Authorization` in `Access-Control-Allow-Headers`). Without them, connecting fails with
  "the server may not allow web pages to reach it". Serving Quatrelle from the mail server's own
  domain avoids the question.
- **Type the provider's session URL.** An account's password goes only to the sites it trusts:
  its address's domain, its server, and the domains allowed in Server settings. The desktop app
  follows a well-known address (`https://example.com/.well-known/jmap`) that redirects to the
  provider's host and asks before trusting another domain; a browser follows the redirect
  itself and drops the password as soon as it changes origin (the Fetch standard), so the
  provider's host answers without it and connecting fails with "<domain> isn't allowed for
  this account". The same message comes when the session names a download or upload address on
  another domain (Fastmail's downloads are on `fastmailusercontent.com`). Type the provider's
  own session URL as the JMAP server (`https://api.fastmail.com/jmap/session`), and add the
  domain the message names in File › Account settings › Server settings › "Other servers
  allowed".
- **Passwords are never kept.** They stay in the tab's memory; after a reload Quatrelle asks
  again. The account list, folder lists and settings are kept in the page's `localStorage`
  (no mail, no passwords). Mail is cached in memory and synced again on each visit.
- **Desktop only:** IMAP/SMTP, Google and Microsoft sign-in, Exchange (Microsoft Graph), opening
  Outlook data files, CalDAV/CardDAV sync, desktop notifications, dragging attachments out.
- **Files are downloads.** Save attachment, Save as and Save as PDF download the file; Print
  downloads the PDF to print from the browser's viewer.
- `?sample` opens the sample mailbox (nothing is kept); `?webgl` forces the WebGL2 renderer.

## Any path works

All URLs in `index.html` are relative (`public_url = "./"` in `apps/quatrelle-web/Trunk.toml`),
so the site works at a domain root, under a prefix (`https://example.com/mail/`) and from a CDN
bucket. The asset names carry a content hash, so they can be cached forever; only `index.html`
needs revalidation.

## Required server settings

- **MIME type:** serve `.wasm` as `application/wasm` (browsers refuse to stream-compile it
  otherwise) and `.js` as `text/javascript`.
- **Compression:** turn on gzip or Brotli for `.wasm`, `.js` and `.html`.
- **Caching:** `Cache-Control: public, max-age=31536000, immutable` on the hashed `.wasm` and
  `.js` files, and `no-cache` on `index.html`.
- **HTTPS:** WebGPU and the clipboard need a secure context (`https://` or
  `http://localhost`); elsewhere the app falls back to WebGL2. A JMAP password should only ever
  travel over HTTPS anyway.
- **Content Security Policy:** if you send one, `connect-src` must include your JMAP server's
  origin, and `script-src` needs `'wasm-unsafe-eval'`.

Local test: `python3 -m http.server 8774` inside the folder, then open http://localhost:8774/
(or http://localhost:8774/?sample).
