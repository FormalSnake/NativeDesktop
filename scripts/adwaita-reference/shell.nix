# Python with GTK 4 and libadwaita from the system's nixpkgs, for board.py.
{ pkgs ? import <nixpkgs> { } }:
pkgs.mkShell {
  nativeBuildInputs = [ pkgs.gobject-introspection pkgs.wrapGAppsHook4 ];
  buildInputs = [
    (pkgs.python3.withPackages (p: [ p.pygobject3 ]))
    pkgs.gtk4
    pkgs.libadwaita
    pkgs.adwaita-icon-theme
    pkgs.adwaita-fonts
  ];
  shellHook = ''
    export XDG_DATA_DIRS=${pkgs.adwaita-icon-theme}/share:${pkgs.gtk4}/share/gsettings-schemas/${pkgs.gtk4.name}:$XDG_DATA_DIRS
  '';
}
