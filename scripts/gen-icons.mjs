// Render public/favicon.svg to PNG app icons (192, 512) for PWA install, using
// the headless Chrome we already have. No image toolchain needed.
import puppeteer from "puppeteer-core";
import { readFileSync, writeFileSync } from "node:fs";

const svg = readFileSync(new URL("../public/favicon.svg", import.meta.url), "utf8");
const browser = await puppeteer.launch({
  executablePath: "/usr/bin/google-chrome",
  headless: "new",
  args: ["--no-sandbox", "--force-device-scale-factor=1"],
});
try {
  for (const size of [192, 512]) {
    const page = await browser.newPage();
    await page.setViewport({ width: size, height: size });
    const html = `<!doctype html><html><body style="margin:0">
      <div style="width:${size}px;height:${size}px">${svg.replace(
        "<svg ",
        `<svg width="${size}" height="${size}" `,
      )}</div></body></html>`;
    await page.setContent(html, { waitUntil: "load" });
    const buf = await page.screenshot({ omitBackground: false, type: "png" });
    const out = new URL(`../public/icon-${size}.png`, import.meta.url);
    writeFileSync(out, buf);
    console.log(`wrote public/icon-${size}.png (${buf.length} bytes)`);
  }
} finally {
  await browser.close();
}
