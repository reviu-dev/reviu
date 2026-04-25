import { env } from "./src/lib/env";

const backendBaseUrl = env.BACKEND_URL.replace(/\/$/, "");

export const desktopDownloadUrls = {
  macAppleSilicon: `${backendBaseUrl}/desktop/update/download/latest/macos/aarch64/Reviu-latest-macos-aarch64.dmg`,
  macIntel: `${backendBaseUrl}/desktop/update/download/latest/macos/x86_64/Reviu-latest-macos-x86_64.dmg`,
  windowsX64: `${backendBaseUrl}/desktop/update/download/latest/windows/x86_64/Reviu-latest-windows-x86_64.exe`,
  windowsArm64: `${backendBaseUrl}/desktop/update/download/latest/windows/aarch64/Reviu-latest-windows-aarch64.exe`,
} as const;

export const latestAppleSiliconDownloadUrl = desktopDownloadUrls.macAppleSilicon;
export const latestMacIntelDownloadUrl = desktopDownloadUrls.macIntel;

export const browserExtensionUrls = {
  firefox: "https://addons.mozilla.org/en-US/firefox/addon/reviu-open-in-app/",
  chrome: "https://chromewebstore.google.com/detail/ofifncflkbaboknlejdkifijpdkhheac",
} as const;
