import { getImage } from "astro:assets";
import type { ImageMetadata } from "astro";

export interface ThemeImageSources {
  darkAvif: string;
  darkWebp: string;
  darkFallback: string;
  lightAvif: string;
  lightWebp: string;
  lightFallback: string;
  width: number;
  height: number;
}

// One image downloads: browser matches prefers-color-scheme source, then best format.
export async function themeImageSources(
  light: ImageMetadata,
  dark: ImageMetadata,
  width: number,
  height: number,
): Promise<ThemeImageSources> {
  const build = async (src: ImageMetadata, format: "avif" | "webp" | "png") =>
    (await getImage({ src, width, height, format })).src;

  const [darkAvif, darkWebp, darkFallback, lightAvif, lightWebp, lightFallback] =
    await Promise.all([
      build(dark, "avif"),
      build(dark, "webp"),
      build(dark, "png"),
      build(light, "avif"),
      build(light, "webp"),
      build(light, "png"),
    ]);

  return {
    darkAvif,
    darkWebp,
    darkFallback,
    lightAvif,
    lightWebp,
    lightFallback,
    width,
    height,
  };
}
