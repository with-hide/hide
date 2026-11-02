import { build } from "vite";
import { copyFile, mkdir, readFile, rename, writeFile } from "node:fs/promises";

const versionSource = await readFile("src/mobile/phoneVersion.ts", "utf8");
const version = Number(versionSource.match(/PHONE_PROTOCOL = (\d+)/)?.[1]);
if (!Number.isSafeInteger(version) || version < 1) throw new Error("phone protocol missing");
await build({ configFile: "phone.config.ts", mode: "loader" });
await rename("phone-dist/phone.html", "phone-dist/index.html");
await build({ configFile: "phone.config.ts", mode: "bundle" });
await rename(`phone-dist/p/${version}/phone-bundle.html`, `phone-dist/p/${version}/index.html`);
await mkdir("phone-dist/m", { recursive: true });
for (const file of ["icon-192.png", "icon-512.png", "apple-touch-icon.png"]) await copyFile(`public/m/${file}`, `phone-dist/m/${file}`);
await copyFile("public/m/sw.js", "phone-dist/sw.js");
const manifest = JSON.parse(await readFile("public/m/manifest.webmanifest", "utf8"));
await writeFile("phone-dist/manifest.webmanifest", JSON.stringify({ ...manifest, id: "/", start_url: "/", scope: "/" }));
await writeFile("phone-dist/protocols.json", JSON.stringify([version]));
