import { info } from "@tauri-apps/plugin-log";
import { downloadFile, getApiJson } from "./WebUtils";
import { getActiveGameCode } from "./AppFunctions";
import { exists, readDir, remove, rename } from "@tauri-apps/plugin-fs";
import { join, resourceDir } from "@tauri-apps/api/path";
import { extractFile, getAllDirs, moveDirItems } from "./FileUtils";
import { Command } from "@tauri-apps/plugin-shell";

export const createWineEnvironment = async () => {
	info("Creating Wine Environment");

	const appDir = await resourceDir();

	const repo = "NelloKudo/spritz-wine-aur";
	const wineDownloadLocation = await join(appDir, "wine.tar.xz");
	let wineExtractLocation = await join(appDir, "wine");
	const winetricksDownloadLocation = await join(
		wineExtractLocation,
		"winetricks",
	);

	info(`Getting JSON for ${repo}`);
	const json = await getApiJson(
		`https://api.github.com/repos/${repo}/releases/latest`,
	);
	const downloadLink = json.assets[0].browser_download_url;
	info(`Downloading from ${downloadLink}`);

	const sha256 = json.assets[0].digest.slice(7);

	info(`Downloading ${repo} to ${wineDownloadLocation}`);
	await downloadFile(downloadLink, wineDownloadLocation);

	// Extract content
	info(`Extracing ${downloadFile} to ${wineExtractLocation}`);
	await extractFile(wineDownloadLocation, wineExtractLocation);

	const wineFolder = (await getAllDirs(wineExtractLocation))[0];

	await moveDirItems(wineFolder, wineExtractLocation);
	remove(wineDownloadLocation);

	// Specifically just for winetricks because it wants to be special and not host the binary on the releases tab of the repo its hosted on
	await downloadFile(
		"https://raw.githubusercontent.com/Winetricks/winetricks/master/src/winetricks",
		winetricksDownloadLocation,
	);

	// Initial Wine Configure (also enable font smoothing)

	// Winetricks configure (install DXVK, vcrun)
};

const launchGame = async () => {
	const game = getActiveGameCode();
};

const winetricksCommand = async (commands: string[]) => {
	const appDir = await resourceDir();
	const winePrefix = await join(appDir, "wine");
	if (!(await exists(winePrefix))) {
		return;
	}

	await Command.create(
		"sh",
		["-c", `./wine/winetricks -q ${commands.join(" ")}`],
		{
			env: {
				WINEPREFIX: winePrefix,
			},
		},
	).execute();
};
