# Native browser HTTP Basic Auth

On macOS, the embedded browser handles a site's HTTP Basic authentication
challenge with an asynchronous native sign-in sheet. It identifies the origin
and realm, masks the password, and supports Cancel and rejected-credential retry.
Credentials are passed directly to WebKit with NSURLCredentialPersistenceNone;
Monitter does not write them to URLs, preferences, logs, or Keychain. WebKit may
reuse authentication within its running network session.

A per-view forwarding delegate preserves Wry navigation policies, load/title
callbacks and download handling. Navigation, hiding, closing and dropping the
pane cancel a pending challenge exactly once. Non-Basic authentication and TLS
trust challenges retain WebKit default handling. Other operating systems retain
their existing browser behavior.

## Local runtime check

Run `python3 scripts/browser-basic-auth-fixture.py`. It prints a loopback port.
Open `http://127.0.0.1:PORT/private` in Monitter. Use dummy username
`monitter-test` and password `basic-auth-test-only`. Check wrong-password retry,
Cancel, successful load (`Basic Auth verified`), reload, navigation to `/public`,
and closing a tab with a prompt open. Stop the fixture afterwards. Its log prints
only request paths and status codes, never credentials or Authorization headers.
