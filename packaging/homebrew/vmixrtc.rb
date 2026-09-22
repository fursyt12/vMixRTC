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
  version "0.1.2"
  sha256 :no_check # при релизе подставьте sha256 из GitHub Release

  url "https://github.com/fursyt12/vMixRTC/releases/download/rust-v#{version}/vMixRTC-macos-universal.zip",
      verified: "github.com/fursyt12/vMixRTC/"
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
