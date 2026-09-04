# Builds Anki from this checkout by reusing the nixpkgs derivation and
# swapping in the local source. Build with: nix build '.?submodules=1'
{ lib, anki, rustPlatform, yarn-berry_4 }:

let
  version = "local";
  src = ./.;

  # Sanity check: verify git submodules are included
  checkSubmodules =
    if !builtins.pathExists "${src}/ftl/core-repo/core" then
      throw ''
        ❌ Git submodules are missing from the source!

        The ftl/core-repo and ftl/qt-repo submodules are required for building.

        Fix: Build with the submodules parameter:
          nix build '.?submodules=1'
      ''
    else true;

  # nixpkgs patches written for the release that no longer apply to main.
  # disable-auto-update: main gained a preference for this instead.
  # yarn-4.14-support: replaced by the local copy below, regenerated for
  # main's .yarnrc.yml.
  droppedPatches = [ "disable-auto-update" "yarn-4.14-support" ];
  keepPatch = patch:
    !lib.any (name: lib.hasInfix name (toString patch)) droppedPatches;

  patches = lib.filter keepPatch anki.patches ++ [ ./yarn-4.14-support.patch ];

  # Regenerate with:
  #   nix run nixpkgs#yarn-berry_4.yarn-berry-fetcher -- missing-hashes yarn.lock > missingHashes.json
  # and the hash with:
  #   nix run nixpkgs#yarn-berry_4.yarn-berry-fetcher -- prefetch yarn.lock missingHashes.json
  missingHashes = ./missingHashes.json;
in
assert checkSubmodules;
anki.overrideAttrs (oldAttrs: {
  inherit version src patches missingHashes;

  # Git dependencies are fetched by rev, so no per-crate hashes are needed.
  cargoDeps = rustPlatform.importCargoLock {
    lockFile = "${src}/Cargo.lock";
    allowBuiltinFetchGit = true;
  };

  yarnOfflineCache = yarn-berry_4.fetchYarnBerryDeps {
    inherit src patches missingHashes;
    hash = "sha256-0vaSB1XNr8DvQ+fjqb84w3cdZzebYYJC71Bm3EfPa7E=";
  };
})
