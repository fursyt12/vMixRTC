# Homebrew cask для vMixRTC.
#
# Как использовать:
#   1. создайте репозиторий `homebrew-vmixrtc` и положите этот файл в `Casks/vmixrtc.rb`;
#   2. обновляйте `version` (и удаляйте `sha256 :no_check`, подставив сумму из релиза);
#   3. установка у пользователя:
#        brew tap fursyt12/vmixrtc https://github.com/fursyt12/homebrew-vmixrtc
#        brew install --cask vmixrtc
#
# `postflight` снимает метку карантина: сборка подписана ad-hoc (без Apple Developer ID),
# и без этого шага Gatekeeper не даст её запустить. Когда появится Developer ID и нотаризация,
# `postflight` можно удалить — подписанные сборки запускаются без обходов.

cask "vmixrtc" do
  version "0.1.8"
  sha256 :no_check # при релизе подставьте sha256 из GitHub Release

  # сборки раздельные: на Apple Silicon ставится arm64-версия, на Intel — x86_64
  on_arm do
    url "https://github.com/fursyt12/vMixRTC/releases/download/rust-v#{version}/vMixRTC-macos-arm64.zip",
        verified: "github.com/fursyt12/vMixRTC/"
  end
  on_intel do
    url "https://github.com/fursyt12/vMixRTC/releases/download/rust-v#{version}/vMixRTC-macos-x86_64.zip",
        verified: "github.com/fursyt12/vMixRTC/"
  end
  name "vMixRTC"
  desc "Cross-platform vMix title controller (widgets, scripts, data providers, NDI)"
  homepage "https://github.com/fursyt12/vMixRTC"

  depends_on macos: ">= :catalina"

  app "vMixRTC.app"

  # Только для ad-hoc подписи: снимаем карантин, иначе macOS блокирует запуск.
  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/vMixRTC.app"],
                   sudo: false
  end

  zap trash: [
    "~/Library/Application Support/vmixrtc",
    "~/Library/Caches/org.vmixrtc.controller",
    "~/Library/Preferences/org.vmixrtc.controller.plist",
    "~/Library/Saved Application State/org.vmixrtc.controller.savedState",
  ]
end
