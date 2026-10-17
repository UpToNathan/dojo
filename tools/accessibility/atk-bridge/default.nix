{ pkgs }:
pkgs.at-spi2-core.overrideAttrs (old: {
  pname = "dojo-at-spi2-core";
  version = "2.62.1";
  src = pkgs.fetchurl {
    url = "https://github.com/GNOME/at-spi2-core/archive/3a214cddffbf345b8c0d0b17e458abbc5ed851b1.tar.gz";
    hash = "sha256-KkReSYDzpcwMbCs1HVeYV+LZBP321lXx9A0aCh1Wjec=";
  };
  patches = (old.patches or []) ++ [ ./endpoint.patch ];
  postInstall = (old.postInstall or "") + ''
    install -Dm644 ../COPYING "$out/share/licenses/dojo-at-spi2-core/COPYING"
  '';
})
