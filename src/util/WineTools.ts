import { info } from "@tauri-apps/plugin-log";
import { downloadFile, getApiJson } from "./WebUtils";
import { getActiveGameCode } from "./AppFunctions";
import { invoke } from "@tauri-apps/api/core";
import { remove } from "@tauri-apps/plugin-fs";
import { join, resourceDir } from "@tauri-apps/api/path";

export const createWineEnvironment = async () => {
	info("Creating Wine Environment");
	
	const appDir = await resourceDir();

	const repo = "NelloKudo/spritz-wine-aur";
	const downloadLocation = await join(appDir, "wine.tar.xz");
	const extractLocation = await join(appDir, "wine");

	info(`Getting JSON for ${repo}`);
	const json = await getApiJson(
		`https://api.github.com/repos/${repo}/releases/latest`,
	);
	const downloadLink = json.assets[0].browser_download_url;
	info(`Downloading from ${downloadLink}`);

	const sha256 = json.assets[0].digest.slice(7);
	info(`Web sha256: ${sha256}`);

	info(`Downloading ${repo} to ${downloadLocation}`);
	await downloadFile(downloadLink, downloadLocation);

	// Extract content
	info(`Extracing ${downloadFile} to ${extractLocation}`);
	await invoke("extract_file", {
		archive: downloadLocation,
		destination: extractLocation,
	});
	remove(downloadLocation);

	// Specifically just for winetricks because it wants to be special and not host the binary on the releases tab of the repo its hosted on
	await downloadFile(
		"https://raw.githubusercontent.com/Winetricks/winetricks/master/src/winetricks",
		"wine/winetricks",
	);
	// Initial Wine Configure (also enable font smoothing)

	// Winetricks configure (install DXVK, vcrun, corefonts)
};

const launchGame = async () => {
	const game = getActiveGameCode();
};
