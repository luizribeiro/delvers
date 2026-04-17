{ pkgs, lib, config, inputs, ... }:

{
  languages.rust.enable = true;

  git-hooks.hooks = {
    rustfmt.enable = true;
    clippy.enable = true;
  };

  enterShell = ''
    install -m 755 .githooks/pre-push .git/hooks/pre-push
  '';
}
