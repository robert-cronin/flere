class FlereConnect < Formula
  desc "SSH companion for the Flere terminal workbench"
  homepage "https://github.com/robert-cronin/flere"
  url "https://github.com/robert-cronin/flere/releases/download/v0.3.8/flere-0.3.8-source.tar.gz"
  version "0.3.8"
  sha256 "d721ef01a17465c3955657a3c1b0eca72ed10c583744a0ddaa4e09905646e87b"
  license all_of: ["MIT", "OFL-1.1"]

  depends_on "rust" => :build

  on_macos do
    depends_on arch: :arm64
  end
  on_linux do
    depends_on arch: :x86_64
  end

  def install
    host = Utils.safe_popen_read("rustc", "--print", "host-tuple").strip
    system "cargo", "fetch", "--locked", "--manifest-path", "companion/Cargo.toml", "--target", host
    system "cargo", "install", "--offline", *std_cargo_args(path: "companion")
    (pkgshare/"licenses").install "LICENSE", "src/assets/fonts/OFL.txt", "src/assets/fonts/LICENSE-Nerd-Fonts"
  end

  def caveats
    <<~EOS
      Update with: brew update && brew upgrade robert-cronin/flere/flere-connect
      In-app Update manages a separate ~/.local/bin installation; use Homebrew here.
      This package builds locally from source. Updates and uninstall preserve Flere
      state and do not stop running sessions. Older macOS runtime acceptance remains
      pending; Intel Macs and Linux ARM are not included in this tap.
    EOS
  end

  test do
    info = JSON.parse(shell_output("#{bin}/flere-connect --build-info"))
    assert_equal "flere-connect", info.fetch("component")
    assert_equal version.to_s, info.fetch("package_version")
    assert_equal "flere-remote-v6", info.fetch("compatibility").fetch("remote_protocol").fetch("current")
    refute_path_exists testpath/".local/state/flere"
  end
end
