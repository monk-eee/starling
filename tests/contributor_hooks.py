"""Exercise the opt-in Git hook in a disposable repository, never in the caller's index."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]


def main():
    with tempfile.TemporaryDirectory(prefix="starling-hook-") as tmp:
        repo = Path(tmp) / "repo"
        repo.mkdir()
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith("GIT_")}
        env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
                   GIT_AUTHOR_NAME="Fixture", GIT_COMMITTER_NAME="Fixture",
                   GIT_AUTHOR_EMAIL="fixture@example.invalid", GIT_COMMITTER_EMAIL="fixture@example.invalid",
                   PRE_COMMIT_HOME=str(Path(tmp) / "hook-cache"))

        def run(*args, failures=()):
            result = subprocess.run(args, cwd=repo, env=env, text=True, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, timeout=120)
            if not failures and result.returncode:
                raise RuntimeError(result.stdout)
            if failures and (not result.returncode or
                             any(f"hook id: {hook}" not in result.stdout for hook in failures)):
                raise RuntimeError(f"Expected rejection by {failures}:\n{result.stdout}")
            return result.stdout.strip()

        run("git", "init", "-q")
        for name in (".pre-commit-config.yaml", ".gitignore", ".gitattributes", "rust-toolchain.toml"):
            shutil.copyfile(ROOT / name, repo / name)
        (repo / "src").mkdir()
        (repo / "examples").mkdir()
        (repo / "changelog.d").mkdir()
        (repo / "Cargo.toml").write_bytes(b'[package]\nname = "fixture"\nversion = "0.1.0"\nedition = "2024"\n')
        (repo / "src/lib.rs").write_bytes(b"pub const VALUE: u8 = 1;\n")
        (repo / "examples/local.yaml").write_bytes(b"name: fixture\n")
        (repo / "changelog.d/report.fixed.md").write_bytes(b"The report opens offline.\n")
        hook = shutil.which("pre-commit")
        runner = [hook] if hook else [sys.executable, "-m", "pre_commit"]
        run(*runner, "validate-config")
        run(*runner, "install")
        run("git", "add", ".")
        run("git", "commit", "-qm", "Valid public change")
        base = run("git", "rev-parse", "HEAD")

        (repo / "src/lib.rs").write_bytes(b"pub const VALUE:u8=2;\n")
        (repo / "examples/personal.yaml").write_bytes(b"name: local-only\n")
        (repo / "changelog.d/wrong.md").write_bytes(b"Bad fragment name.\n")
        (repo / "note.txt").write_bytes(b"Trailing space \n")
        (repo / "key.txt").write_bytes(b"-----BEGIN " + b"PRIVATE KEY-----\nnot a real key\n")
        run("git", "add", ".")
        run("git", "add", "--force", "examples/personal.yaml")
        # A clean working copy must not disguise the unformatted staged blob.
        working = b"pub const VALUE: u8 = 2;\n"
        (repo / "src/lib.rs").write_bytes(working)
        run("git", "commit", "-qm", "Invalid change",
            failures=("staged-diff", "rustfmt", "local-only-paths", "private-key-header", "fragment-name"))
        if run("git", "rev-parse", "HEAD") != base or (repo / "src/lib.rs").read_bytes() != working:
            raise RuntimeError("The rejected commit changed HEAD or failed to restore unstaged work")
        print("Valid commit accepted; all five checks rejected bad staged files; unstaged work preserved.")


if __name__ == "__main__":
    main()
