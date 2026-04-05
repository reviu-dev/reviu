import { env } from "./src/lib/env";

const backendBaseUrl = env.BACKEND_URL.replace(/\/$/, "");

export const desktopDownloadUrls = {
  macAppleSilicon: `${backendBaseUrl}/desktop/update/download/latest/macos/aarch64/Reviu-latest-macos-aarch64.dmg`,
  macIntel: `${backendBaseUrl}/desktop/update/download/latest/macos/x86_64/Reviu-latest-macos-x86_64.dmg`,
} as const;

export const latestAppleSiliconDownloadUrl = desktopDownloadUrls.macAppleSilicon;
export const latestMacIntelDownloadUrl = desktopDownloadUrls.macIntel;
