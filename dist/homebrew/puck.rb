# frozen_string_literal: true

# TEMPLATE ONLY — release workflow should regenerate sha256/urls from manifest.json.
# brew tap VerburgtJimmy/puck https://github.com/VerburgtJimmy/puck
# brew install puck

class Puck < Formula
  desc "Native PHP package installer (Composer-compatible vendor/)"
  homepage "https://github.com/VerburgtJimmy/puck"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/VerburgtJimmy/puck/releases/download/v0.1.0/puck-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/VerburgtJimmy/puck/releases/download/v0.1.0/puck-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/VerburgtJimmy/puck/releases/download/v0.1.0/puck-aarch64-unknown-linux-musl.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/VerburgtJimmy/puck/releases/download/v0.1.0/puck-x86_64-unknown-linux-musl.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    bin.install "puck"
  end

  test do
    assert_match(/puck/, shell_output("#{bin}/puck --help"))
  end
end
