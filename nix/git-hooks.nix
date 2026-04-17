{
  git-hooks,
  pkgs,
  src,
}:
git-hooks.lib.${pkgs.stdenv.hostPlatform.system}.run {
  inherit src;
  package = pkgs.prek;
  configPath = ".git-hooks.nix.pre-commit-config.yaml";

  hooks = {
    alejandra.enable = true;

    check-added-large-files.enable = true;
    check-case-conflicts.enable = true;
    check-merge-conflicts.enable = true;
    check-toml.enable = true;
    check-yaml.enable = true;
    detect-private-keys.enable = true;
    end-of-file-fixer.enable = true;

    mixed-line-endings = {
      enable = true;
      args = ["--fix=auto"];
    };

    trim-trailing-whitespace.enable = true;

    cargo-fmt = {
      enable = true;
      name = "cargo fmt";
      entry = "${pkgs.cargo}/bin/cargo fmt --all";
      files = "(^|/).+\\.rs$|(^|/)Cargo\\.toml$";
      language = "system";
      pass_filenames = false;
    };

    cargo-lock-sync = {
      enable = true;
      name = "cargo lock sync";
      entry = "${pkgs.cargo}/bin/cargo metadata --locked --format-version=1";
      files = "(^|/)(Cargo\\.toml|Cargo\\.lock)$";
      language = "system";
      pass_filenames = false;
    };

    codespell = {
      enable = true;
      name = "codespell";
      entry = "${pkgs.codespell}/bin/codespell --ignore-words-list=ser";
      language = "system";
      pass_filenames = true;
    };

    check-github-workflows = {
      enable = true;
      name = "check github workflows";
      entry = "${pkgs.actionlint}/bin/actionlint";
      files = "^\\.github/workflows/.*\\.ya?ml$";
      language = "system";
      pass_filenames = false;
    };

    shfmt = {
      enable = true;
      files = "^scripts/.*\\.sh$";
    };

    shellcheck = {
      enable = true;
      files = "^scripts/.*\\.sh$";
    };
  };
}
