class Zcc < Formula
  desc "Zed <-> Claude Code integration: send editor selection to Claude Code via /ide"
  homepage "https://github.com/noah-hrbth/zed-claude-code"
  version "0.2.0"
  license "MIT"

  on_arm do
    url "https://github.com/noah-hrbth/zed-claude-code/releases/download/v#{version}/zcc-#{version}-aarch64-apple-darwin.tar.gz"
    sha256 "55fd6b33452f3d4816f67ebf50c97d3a7b0a1d103eb9cb5dd91bea89e4e30e51"
  end

  on_intel do
    url "https://github.com/noah-hrbth/zed-claude-code/releases/download/v#{version}/zcc-#{version}-x86_64-apple-darwin.tar.gz"
    sha256 "072f9a405c4957638869bbf7f9575fc571c65c22faf7deaba2c5bdd3a8c1938b"
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
