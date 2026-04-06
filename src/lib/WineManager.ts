import { appDataDir, join } from "@tauri-apps/api/path";
import { ComponentData, WineComponent, WineModule } from "../types";
import { exists, remove, removeDir, rename } from "./Fs";
import { fetch } from "@tauri-apps/plugin-http";
import { downloadFile } from "../util/WebUtils";
import { extractFile } from "../util/FileUtils";
import { error, info } from "@tauri-apps/plugin-log";
import { executeLocalCommand, executeShellCommand } from "../util/AppFunctions";

// Components that get regular updates (i.e. wine, dxvk)
const components: WineComponent[] = [
	{
		componentName: "wine",
		extractTo: "wine",
		saveTo: "wine.tar.xz",
		postInstall: async () => {
			// Set up the wineprefix for use and install all additional wine modules
			if (!(await exists(await join("wine", "drive_c")))) {
				// Initial wineserver startup
				await wineCommand("-i", "wineboot");
				await wineCommand("--wait", "wineserver");
				await wineCommand("-k", "wineserver");

				await installWineModules();
			}
		},
	},
	{
		componentName: "dxvk",
		extractTo: "dxvk",
		saveTo: "dxvk.tar.gz",
		postInstall: async () => {
			// Move contents into Wine's drive_c directory and add registry keys to associate the new DLL files with wine
			const appData = await appDataDir();

			await executeShellCommand(
				`mv -v ${appData}/dxvk/x64/*.dll ${appData}/wine/drive_c/windows/system32`,
			);
			await executeShellCommand(
				`mv -v ${appData}/dxvk/x32/*.dll ${appData}/wine/drive_c/windows/syswow64`,
			);

			const dllNames = ["d3d9", "d3d10core", "d3d11", "dxgi"] as const;
			await Promise.all(
				dllNames.map(async (dll) => {
					await registerNewDLL(dll);
				}),
			);

			// Remove Temporary Directory
			await removeDir("dxvk");
		},
	},
	{
		componentName: "jadeite",
		extractTo: "jadeite",
		saveTo: "jadeite.zip",
		postInstall: async () => {
			// Run the telemetry blocker script. Will ask for user admin permission if the telemetry blocking hasn't been applied before
			await executeLocalCommand("jadeite/block_analytics.sh");
		},
	},
	{
		// While not used right now, it will for certain be used in future games or game updates as DX12 becomes the industry standard. Best to install and stay ahead of updated
		componentName: "vkd3d",
		extractTo: "vkd3d-temp",
		saveTo: "vkd3d.tar.zst",
		postInstall: async () => {
			// Run setup_vkd3d_proton.sh to automate the installation process
			await executeLocalCommand("vkd3d-temp/setup_vkd3d_proton.sh", "install", {
				WINEPREFIX: await winePrefix(),
			});

			await removeDir("vkd3d");
		},
	},
] as const;

// Components that do not need to be updated (i.e. Visual C++ Redistributable)
const wineModules: WineModule[] = [
	{
		name: "vcrun2026-x64",
		downloadLink: "https://aka.ms/vc14/vc_redist.x64.exe",
		moduleType: "exe",
	},
	{
		name: "vcrun2026-x86",
		downloadLink: "https://aka.ms/vs/17/release/vc_redist.x86.exe",
		moduleType: "exe",
	},
	{
		name: "d3dcompiler_47.dll",
		downloadLink:
			"https://raw.githubusercontent.com/mozilla/fxc2/master/dll/d3dcompiler_47.dll",
		moduleType: "dll64",
	},
	{
		name: "d3dcompiler_47.dll",
		downloadLink:
			"https://raw.githubusercontent.com/mozilla/fxc2/master/dll/d3dcompiler_47_32.dll",
		moduleType: "dll32",
	},
] as const;

/**
 * Update All Components in the wine install
 */
export const updateWineComponents = async (): Promise<void> => {
	info(`${await appDataDir()}`);
	for (const component of components) {
		try {
			info(`Installing ${component.componentName}`);

			const assetURL = `https://raw.githubusercontent.com/elysiae-project/components/refs/heads/main/components/${component.componentName}.json`;
			const response = await fetch(assetURL);
			if (response.status === 200) {
				const json: ComponentData[] = await response.json();
				await downloadFile(json[0].download_url, component.saveTo);

				// Extract file
				await extractFile(component.saveTo, component.extractTo);

				if (typeof component.postInstall !== "undefined") {
					await component.postInstall();
				}
			} else {
				throw new Error("Endpoint returned non-OK response code");
			}
		} catch (e) {
			error(`updateWineComponents ${e}`);
		}
	}
	info("Wine Component Download Complete");
};

/**
 * Install additional Windows programs/libraries required for the programs that Elysiae runs
 */
const installWineModules = async (): Promise<void> => {
	await Promise.all(
		wineModules.map(async (module) => {
			info(`Installing ${module.name}`);

			const filename = module.downloadLink.split("/").pop() as string;
			await downloadFile(module.downloadLink, filename);

			if (module.moduleType === "exe") {
				await runExeWithWine(filename, "/install /quiet /norestart");
				await remove(filename);
			} else {
				if (module.moduleType === "dll64") {
					const sys32 = await join("wine", "drive_c", "windows", "system32");
					await rename(filename, `${sys32}/${filename}`);
				} else {
					// moduleType === "dll32"
					const syswow = await join("wine", "drive_c", "windows", "syswow64");
					await rename(filename, `${syswow}/${filename}`);
				}
				await registerNewDLL(filename.split(".")[0]);
			}
		}),
	);
	info("Wine Module Download Complete");
};

/**
 * Run a command in the wine package or in the scope of a wine environment
 * @param args Command arguments
 * @param binary Which wine binary you want to run
 */
export const wineCommand = async (
	args: string,
	binary: "wine" | "wineboot" | "wineserver" = "wine",
): Promise<void> => {
	const prefix = await winePrefix();
	const appData = await appDataDir();
	const wineLib = await join(appData, "wine", "lib");
	const wineLib64 = await join(appData, "wine", "lib64");

	await executeLocalCommand(`wine/bin/${binary}`, args, {
		WINEPREFIX: prefix,
		WINEARCH: "win64",
		WINEFSYNC: "1",
		LD_LIBRARY_PATH: `${wineLib64}:${wineLib}:${wineLib64}/wine/x86_64-unix:${wineLib}/wine/i386-unix`,
		DXVK_ASYNC: "1",
	});
};

/**
 * Execute a Windows executable (.exe) with Wine
 * @param path path to the executable
 * @param args Any additional arguments the executable may have
 */
export const runExeWithWine = async (
	path: string,
	args?: string,
): Promise<void> => {
	const appData = await appDataDir();
	const fullPath = await join(appData, path);

	await wineCommand(`${fullPath} ${typeof args !== "undefined" ? args : ""}`);
};

export const runExeWithJadeite = async (path: string): Promise<void> => {
	const appData = await appDataDir();
	const fullJadeitePath = await join(appData, "jadeite", "jadeite.exe");
	const fullExePath = await join(appData, path);
	await wineCommand(`${fullJadeitePath} ${fullExePath}`);
};

/**
 * Adds a DLL to the wine registry
 * @param dllName name of the DLL. No path needed, just the name
 */
const registerNewDLL = async (dllName: string): Promise<void> => {
	await wineCommand(
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v ${dllName} /t REG_SZ /d native /f`,
	);
};

/**
 * @returns weather or not a wine environment exists (checks if the drive_c folder exists, which indicates that a wine environment is present)
 */
export const wineEnvAvailable = async (): Promise<boolean> => {
	const winePath = await winePrefix();
	const driveC = await join(winePath, "drive_c");

	if ((await exists(winePath)) && (await exists(driveC))) {
		return true;
	}
	return false;
};

/**
 * @returns Path to the wine prefix directory
 */
export const winePrefix = async (): Promise<string> => {
	return new Promise((resolve) => {
		appDataDir().then((appData) => {
			join(appData, "wine").then((res) => {
				resolve(res as string);
			});
		});
	});
};
