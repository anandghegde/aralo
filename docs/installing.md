# Installing

Aralo needs macOS 14 or later, on Apple silicon or Intel.

> There is no signed release yet. Until there is, build the app from source:
> see [Contributing](contributing.md).

## With Homebrew

```sh
brew install --cask anandghegde/aralo/aralo
```

The cask installs `Aralo.app` into `/Applications`. Aralo then updates itself
(below), so `brew upgrade` leaves it alone.

`brew uninstall --zap --cask aralo` also removes Aralo's settings and caches
in `~/Library`. It never removes your snippet library (`~/Aralo`, or wherever
you moved it), and API keys stay in the keychain until you delete them there.

## From the DMG

Download `Aralo-<version>.dmg` from the
[releases page](https://github.com/anandghegde/aralo/releases), open it and
drag Aralo to Applications. Every release is signed with a Developer ID and
notarised by Apple, so it opens without a Gatekeeper warning.

To check a download against the release's `SHA256SUMS`:

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing
```

## Updates

Aralo checks GitHub for a new version with [Sparkle](https://sparkle-project.org),
and installs an update only if it is signed with the project's update key.
*Check for Updates…* in the menu bar item checks at once.

In local-only mode Aralo checks for nothing: local-only means no network,
updates included. Switch local-only off to update, or install the new
version by hand.
