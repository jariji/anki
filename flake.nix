{
  description = "Anki - powerful, intelligent flashcards";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages.default = pkgs.callPackage ./default.nix { };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];
          shellHook = ''
            echo "======================================"
            echo "Build with submodules: nix build '.?submodules=1'"
            echo ""
            echo "Git workflow:"
            echo "  Feature branches are rebased on nix (have nix + feature)"
            echo "  Work directly on feature branch, then push feature commits only:"
            echo "    git checkout -b <feature>-pr upstream/main"
            echo "    git cherry-pick <feature-commits>"
            echo "    git push origin <feature>-pr"
            echo "======================================"
          '';
        };
      });
}
