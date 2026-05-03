class Zcc < Formula
  desc "Zed <-> Claude Code integration: send editor selection to Claude Code via /ide"
  homepage "https://github.com/noah-hrbth/zed-claude-code"
  version "0.2.0"
  license "MIT"

  on_arm do
    url "https://github.com/noah-hrbth/zed-claude-code/releases/download/v0.2.0/zcc-0.2.0-aarch64-apple-darwin.tar.gz"
    sha256 "0362590e476ee9e00eeff74bd3801cb9b82d6e8cb072eb5d28fb1a8227213795"
  end

  on_intel do
    url "https://github.com/noah-hrbth/zed-claude-code/releases/download/v0.2.0/zcc-0.2.0-x86_64-apple-darwin.tar.gz"
    sha256 "725bb0fada08532d0e07b6aa9fafa2b60cd2c86e56f440866c474fe90b4ec615"
  end

  def install
    bin.install "zcc"
  end

  def caveats
    <<~EOS
      To finish setup, run:
        zcc install

      That merges an entry into your Zed tasks.json and keymap.json
      so Cmd+L sends the editor selection to Claude Code.

      Originals are backed up alongside each file as *.bak.
    EOS
  end

  test do
    assert_match "zcc #{version}", shell_output("#{bin}/zcc --version")
    assert_match "Send the current editor selection", shell_output("#{bin}/zcc send --help")
    assert_match "zcc doctor", shell_output("#{bin}/zcc doctor")
  end
end
