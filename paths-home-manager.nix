{ lib, ... }:
{
  options.dojo.root = lib.mkOption {
    type = lib.types.str;
    description = "Absolute path of the Dojo checkout that the Home Manager modules run.";
  };
}
