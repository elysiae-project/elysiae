import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const packageJson = JSON.parse(readFileSync("package.json", "utf-8"));
if (
	!packageJson.packageManager ||
	typeof packageJson.packageManager !== "string" ||
	!packageJson.packageManager.startsWith("yarn@")
) {
	console.log("No packageManager found in package.json");
	process.exit(1);
}
const version = packageJson.packageManager.split("@")[1].split("+")[0];

const url = `https://repo.yarnpkg.com/${version}/packages/yarnpkg-cli/bin/yarn.js`;
const dest = `flatpak-node/corepack-home/v1/yarn/${version}`;

console.log(`Fetching yarn.js for version ${version}...`);
console.log(`URL: ${url}`);

const response = await fetch(url);
if (!response.ok) {
	console.error(
		`Failed to fetch yarn.js: ${response.status} ${response.statusText}`,
	);
	process.exit(1);
}

const buffer = Buffer.from(await response.arrayBuffer());
const sha512 = createHash("sha512").update(buffer).digest("hex");

console.log(`SHA-512: ${sha512}`);

const corepackMeta = JSON.stringify({
	locator: { name: "yarn", reference: version },
	bin: ["yarn", "yarnpkg"],
	hash: `sha512.${sha512}`,
});

const manifest = [
	{
		type: "file",
		url,
		sha512,
		"dest-filename": "yarn.js",
		dest,
	},
	{
		type: "inline",
		contents: corepackMeta,
		"dest-filename": ".corepack",
		dest,
	},
	{
		type: "inline",
		contents: "{}",
		"dest-filename": "package.json",
		dest: "flatpak-node",
	},
	{
		type: "shell",
		commands: [
			`mkdir -- "flatpak-node/corepack-shims"`,
			`ln -sT -- "$(command -v corepack)" "flatpak-node/corepack-shims/corepack"`,
			`COREPACK_HOME="\${PWD:?}/flatpak-node/corepack-home" COREPACK_ENABLE_NETWORK="0" PATH="\${PWD:?}/flatpak-node/corepack-shims\${PATH:+:\${PATH:?}}" corepack --version`,
			`COREPACK_HOME="\${PWD:?}/flatpak-node/corepack-home" COREPACK_ENABLE_NETWORK="0" PATH="\${PWD:?}/flatpak-node/corepack-shims\${PATH:+:\${PATH:?}}" corepack enable`,
			`COREPACK_HOME="\${PWD:?}/flatpak-node/corepack-home" COREPACK_ENABLE_NETWORK="0" PATH="\${PWD:?}/flatpak-node/corepack-shims\${PATH:+:\${PATH:?}}" corepack install`,
		],
	},
	{
		type: "script",
		commands: [
			"corepack enable yarn",
			"yarn config set enableNetwork false",
			"yarn config set enableOfflineMode true",
			`yarn config set globalFolder -- "\${FLATPAK_BUILDER_BUILDDIR:?}/flatpak-node/yarn-berry"`,
			`yarn plugin import "\${FLATPAK_BUILDER_BUILDDIR:?}/flatpak-node/flatpak-yarn.js"`,
			`yarn convertToZip "\${FLATPAK_BUILDER_BUILDDIR:?}/flatpak-node/corepack-home/v1/yarn/${version}/yarn.js"`,
		],
		dest: "flatpak-node",
	},
];

const outFile = "download-sources/corepack.json";
writeFileSync(outFile, JSON.stringify(manifest, null, 2) + "\n");
console.log(`Written to ${outFile}`);
