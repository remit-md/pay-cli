{
  description = "pay — USDC payment infrastructure for AI agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        version = "0.2.2";
        platformMap = {
          "x86_64-linux" = { artifact = "pay-linux-amd64"; sha256 = "sha256-5/dM6ly3ehmhlcTfMvuCl4jX4ExU43NWJ/z4WHtAIno="; };
          "aarch64-linux" = { artifact = "pay-linux-arm64"; sha256 = "sha256-9JxVXFj1nTFL5UHk7WQesOpmYHzFwDfS0BugPlPcT0M="; };
          "x86_64-darwin" = { artifact = "pay-macos-amd64"; sha256 = "sha256-JIaYtz39DUnLc5qE923E6h5cw11kEbcBKFvMIHyy4fU="; };
          "aarch64-darwin" = { artifact = "pay-macos-arm64"; sha256 = "sha256-aKaCszAOYpOCOaHAbJmt+NaqeBS+JyY8G6UdSSLFnMY="; };
        };
        platform = platformMap.${system} or (throw "Unsupported system: ${system}");
      in {
        packages.default = pkgs.stdenv.mkDerivation {
          pname = "pay";
          inherit version;
          src = pkgs.fetchurl {
            url = "https://github.com/pay-skill/pay-cli/releases/download/v${version}/${platform.artifact}";
            sha256 = platform.sha256;
          };
          dontUnpack = true;
          installPhase = ''
            install -Dm755 $src $out/bin/pay
          '';
          meta = with pkgs.lib; {
            description = "CLI for pay — USDC payment infrastructure for AI agents";
            homepage = "https://pay-skill.com";
            license = licenses.mit;
            platforms = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
          };
        };
      });
}
