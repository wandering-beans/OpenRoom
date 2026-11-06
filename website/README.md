# OpenRoom distribution website

Static source for the existing public ChatGPT Site at https://deepdive.boats.
No build step, API key, paid service, database, or scheduled automation is needed.

Run `node website/test-releases.mjs` and `node website/test-downloads.mjs` from
the repository root. Serve `website/` over HTTP to preview the ES modules.

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

To publish, open the existing Site source with the Sites workflow helper.
Copy the public files (`index.html`, `styles.css`, `script.js`, `releases.mjs`,
`downloads.mjs`, and `assets/`) to its root and `dist/`. Preserve
`.openai/hosting.json`, Site identity, and audience. Copy the page to
`ja/index.html` and `dist/ja/index.html` to preserve the prior Japanese URL.
Validate, package, save, and deploy with Sites. Do not copy local QA output.

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
