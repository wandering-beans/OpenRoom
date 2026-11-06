# OpenRoom distribution website

Source for the existing public ChatGPT Site at https://deepdive.boats.
A small Worker serves the existing pages and records download clicks in Sites D1.
No external analytics service, API key, paid upgrade, or scheduled automation is needed.

Run `npm test` from `website/` with Node 24 to build and test the page, download
consent, release selection, and counter against a temporary SQLite database.
The Worker has no runtime dependencies. `npm ci` is needed only for the schema
tools; after changing `db/schema.ts`, run `npm run db:generate` and commit the
generated SQL and metadata in `drizzle/`. Never edit applied migrations.

`releases.mjs` reads public GitHub releases on page load and when the page
becomes visible again. It chooses Windows x64 installers and macOS Apple
Silicon/universal DMG or ZIP files independently, by publication date,
including prereleases and excluding drafts, source bundles, signatures,
updater archives, and incompatible Windows architectures. It follows
pagination when one platform is missing. On GitHub errors/rate limits, it
uses the releases page instead of presenting stale downloads as the latest.
Future assets must retain the names covered by the selector, such as
`OpenRoom-…-windows-x64-setup.exe` and `OpenRoom-…-macos-arm64.zip`.

`downloads.mjs` requires an unchecked-by-default agreement to the repository's
`TERMS.md` before enabling a download. Submission also checks consent. Nothing
is persisted, and a back-forward cache restore clears consent. This is a
website interaction requirement, not access control over public GitHub assets.

After consent and only for an available release asset, the form sends a
same-origin `POST /api/download-click/windows` or `/api/download-click/macos`
with `keepalive` before navigating. Release-list fallback links do not count.
Tracking failure never blocks the download; requests are not retried to avoid
double counting. Offline requests, blockers, or exhausted service limits can
cause undercounting. Counts are clicks, not completed downloads or unique people.

The `DB` binding stores only two platform totals in `download_counts`; no IP
addresses, user agents, cookies, or per-visitor records are stored by this counter.
An atomic UPSERT prevents lost increments. Cross-origin browser requests and
unknown platforms are rejected. Like any public click endpoint, scripted clients
can imitate a click; this is a lightweight usage counter, not fraud-proof analytics.
Inspect counts through the authenticated Sites database viewer or Sites database
tools. There is no public totals endpoint. Before the first click, a platform has
no row and its count is zero. Sum `clicks` across the rows for the overall total.

To publish, open the existing Site source with the Sites workflow helper. Copy
the contents of `website/` to the checkout, excluding `node_modules`, `dist`, and
local QA output. Preserve the Site identity and audience. The hosting manifest
uses `d1: "DB"` and no `static` setting. Run `node build.mjs` and the tests, then
package, save, and deploy with Sites. Sites applies the Drizzle migrations.
The build embeds an explicit allowlist of assets in `dist/server/index.js` and
keeps `/ja/` working. Database contents survive subsequent deployments.

Screenshots show the installed Windows OpenRoom app with a dedicated demo
catalog and public Unsplash samples. They are actual app captures, not mockups.
No personal photos, catalog contents, paths, or metadata are published.
The icon comes from `assets/branding/openroom-brand-kit/`.

Sample photos inside the screenshots are used under the
[Unsplash License](https://unsplash.com/license), with these image IDs:

- `photo-1519681393784-d120267933ba` (night sky and mountain)
- `photo-1441974231531-c6227db76b6e` (forest light)
- `photo-1472396961693-142e6e269027` (deer and mountain)

Product descriptions and requirements follow the repository README and the
release notes. The terms preserve the GPL rights of recipients.
