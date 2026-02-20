import { info } from "@tauri-apps/plugin-log";
import { downloadFile, getApiJson } from "./WebUtils";
import { getActiveGameCode } from "./AppFunctions";
import { exists, remove } from "@tauri-apps/plugin-fs";
import { join, resourceDir } from "@tauri-apps/api/path";
import { extractFile, getAllDirs, moveDirItems } from "./FileUtils";
import { Command } from "@tauri-apps/plugin-shell";

export const createWineEnvironment = async () => {
	const appDir = await resourceDir();
	const winetricksDownloadLocation = await join(appDir, "wine", "winetricks");

	const repoAssets = [
		{
			repo: "NelloKudo/spritz-wine-aur",
			downloadLocation: await join(appDir, "wine.tar.xz"),
			extractLocation: await join(appDir, "wine"),
		},
		{
			// While not entirely necessary, I think that current games could be updated to use DX12, and future games will most certainly use DX12
			// Good for futureproofing, also provides a very gaming-focused wine prefix for yoohoo
			repo: "HansKristian-Work/vkd3d-proton",
			downloadLocation: await join(appDir, "vkd3d-proton.tar.xz"),
			extractLocation: await join(appDir, "vkd3d-temp"),
		},
	];

	if (!(await exists(repoAssets[0].extractLocation))) {
		for (let i = 0; i < repoAssets.length; i++) {
			const repo = repoAssets[i].repo;
			const downloadLocation = repoAssets[i].downloadLocation;
			const extractLocation = repoAssets[i].extractLocation;

			// Get the download link to the asset
			const json = await getApiJson(
				`https://api.github.com/repos/${repo}/releases/latest`,
			);
			const downloadLink = json.assets[0].browser_download_url;
			info(`Downloading from ${downloadLink}`);

			// TODO: Hash validation
			const sha256 = json.assets[0].digest.slice(7);

			// Download file
			await downloadFile(downloadLink, downloadLocation);

			// Extract content
			await extractFile(downloadLocation, extractLocation);

			// Get path to the "parent folder" of the extracted directory and move to the proper extract location
			const folder = (await getAllDirs(extractLocation))[0];
			await moveDirItems(folder, extractLocation);

			// Remove downloaded asset
			remove(downloadLocation);
		}
	}
	if (!(await exists(winetricksDownloadLocation))) {
		// Winetricks has a direct link available to download, rather than a release on github
		await downloadFile(
			"https://raw.githubusercontent.com/Winetricks/winetricks/master/src/winetricks",
			winetricksDownloadLocation,
		);
	}

	// Use Winetricks to install install DXVK and vcrun
	// This would resolve to 'winetricks -q {vcrun2022,dxvk}'.
	// The command will run once for each value within the braces. in this case, it will install vcrun2022 and dxvk
	// Winetricks also generates the wineprefix folder/content automatically, so there's no need to go out of the way to generate one
	await winetricksCommand(["{vcrun2022,dxvk}"]);

	// Install vkd3d-proton
	await Command.create("sh", [
		"-c",
		`mv ${repoAssets[1].extractLocation}/x64/* ${repoAssets[0].extractLocation}/drive_c/windows/system32/`,
		"&&",
		`mv ${repoAssets[1].extractLocation}/x86/* ${repoAssets[0].extractLocation}/drive_c/windows/syswow64`,
	]).execute();
	await remove(repoAssets[1].extractLocation);

	// d3d12
	await wineCommand([
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v d3d12 /t REG_SZ /d native /f`,
	]);

	// d3d12core
	await wineCommand([
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v d3d12core /t REG_SZ /d native /f`,
	]);
};

const launchGame = async () => {
	const game = getActiveGameCode();
};

const wineCommand = async (commands: string[]) => {
	const appDir = await resourceDir();
	const winePrefix = await join(appDir, "wine");
	if (!(await exists(winePrefix))) {
		await createWineEnvironment();
	}

	await Command.create("sh", ["-c", `./wine/bin/wine ${commands.join(" ")}`], {
		env: {
			WINEPREFIX: winePrefix,
		},
	}).execute();
};

const winetricksCommand = async (commands: string[]) => {
	const appDir = await resourceDir();

	const winePrefix = await join(appDir, "wine");
	if (!(await exists(winePrefix))) {
		await createWineEnvironment();
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
