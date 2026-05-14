import { appDataDir, join } from "@tauri-apps/api/path";
import {
	WineComponentData,
	AppModules,
	ComponentData,
	WineComponent,
	WineModule,
	ModuleData,
} from "../types";
import { exists, extractFile, readDir, remove, removeDir, rename } from "./Fs";
import { fetch } from "@tauri-apps/plugin-http";
import { downloadFile, getApiJson } from "../util/WebUtils";
import { error, info } from "@tauri-apps/plugin-log";
import { executeLocalBinary, executeShellCommand } from "../util/AppFunctions";
import { getOption, setOption } from "../util/Settings";

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
			// Move the dxvk DLLs into the wine prefix and register them in the registry
			const dirs = [
				{
					initialDirName: "x64",
					finalDirName: "system32",
				},
				{
					initialDirName: "x32",
					finalDirName: "syswow64",
				},
			] as const;

			dirs.map(async (dirInfo) => {
				const destPath = await join(
					"wine",
					"drive_c",
					"windows",
					dirInfo.finalDirName,
				);
				const sourceFolder = await join("dxvk", dirInfo.initialDirName);
				rename(sourceFolder, destPath);
			});
			await removeDir("dxvk");
		},
	},
	{
		componentName: "jadeite",
		extractTo: "jadeite",
		saveTo: "jadeite.zip",
		postInstall: async () => {
			// Run the telemetry blocker script. Will ask for user admin permission if the telemetry blocking hasn't been applied before
			await executeLocalBinary("jadeite/block_analytics.sh");
		},
	},
	/*{
		// While not used right now, it will for certain be used in future games or game updates as DX12 gets adopted by these games
		componentName: "vkd3d",
		extractTo: "vkd3d",
		saveTo: "vkd3d.tar.zst",
		postInstall: async () => {
			// Run setup_vkd3d_proton.sh to automate the installation process
			await executeLocalBinary("vkd3d/setup_vkd3d_proton.sh", "install", {
				WINEPREFIX: await winePrefix(),
			});

			await removeDir("vkd3d");
		},
	},*/
] as const;

// Components that do not need to be updated (i.e. Visual C++ Redistributable)
// TODO: Review which of these modules are actually needed
const wineModules: WineModule[] = [
	{
		name: "vcrun-x64",
		downloadLink: "https://aka.ms/vc14/vc_redist.x64.exe",
		moduleType: "exe",
	},
	{
		name: "vcrun-x86",
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
export const updateAllWineComponents = async (): Promise<void> => {
	// A bit of a compromise to allow for individual component updates.
	// The json is only referenced in updateWineComponent(), and even then the
	// function only gets the respective module index by searching for the name of the module
	const modules: AppModules[] = ["wine", "dxvk", "jadeite"];
	for (let i = 0; i < modules.length; i++) {
		try {
			await updateWineComponent(modules[i]);
		} catch (e) {
			error(`updateAllWineComponents: ${e}`);
			return;
		}
	}
	info("Wine Component Download Complete");
};

/**
 * Updates a specified wine component
 * @param componentName A valid wine component name
 */
export const updateWineComponent = async (
	componentName: AppModules,
): Promise<void> => {
	const index = components.findIndex(
		(data) => data.componentName === componentName,
	);
	const data = components[index];
	try {
		info(`Installing/Updating ${data.componentName}`);
		const assetURL = `https://raw.githubusercontent.com/elysiae-project/components/refs/heads/main/components/${data.componentName}.json`;
		const assetResponse = await fetch(assetURL);
		if (assetResponse.status === 200) {
			// Download file to download location
			const json: ComponentData = (await assetResponse.json())[0];
			await downloadFile(json.download_url, data.saveTo);

			// Extract downloaded file to folder location
			await extractFile(data.saveTo, data.extractTo);

			// Perform postinstall actions, if any exist
			if (typeof data.postInstall !== "undefined") {
				await data.postInstall();
			}

			// Update the wine module tracker
			await updateModuleTracker(data.componentName, json.tag);
		} else {
			throw new Error(
				`installWineComponent: Endpoint "${assetURL}" returned non-zero access code (${assetResponse.status})`,
			);
		}
	} catch (e) {
		error(`installWineComponent: ${e}`);
		return;
	}
	info(`Installation/Update of ${data.componentName} succeeded.`);
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

	await executeLocalBinary(`wine/bin/${binary}`, args, {
		WINEPREFIX: prefix,
		WINEARCH: "win64",
		LD_LIBRARY_PATH: `${wineLib64}:${wineLib}:${wineLib64}/wine/x86_64-unix:${wineLib}/wine/i386-unix`,
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
	const winePrefixPath = await winePrefix();
	const driveC = await join(winePrefixPath, "drive_c");

	return (
		(await exists(winePrefixPath)) &&
		(await exists(driveC)) &&
		(await exists("jadeite"))
	);
};

/**
 * @returns Path to the wine prefix directory
 */
export const winePrefix = async (): Promise<string> => {
	return new Promise((resolve) => {
		appDataDir().then((appData) => {
			join(appData, "wine").then((res) => {
				resolve(res);
			});
		});
	});
};

export const updateModuleTracker = async (
	module: AppModules,
	newVersion: string,
) => {
	const current = await getOption<WineComponentData>("installedComponents");
	current[module] = newVersion;
	await setOption("installedComponents", current);
};

export const getModuleVersion = async (
	module: AppModules | undefined = undefined,
): Promise<WineComponentData | string | null> => {
	return new Promise((resolve, reject) => {
		getOption<WineComponentData>("installedComponents")
			.then((data) => {
				if (typeof module === undefined) {
					resolve(data);
				}
				resolve(data[module as AppModules]);
			})
			.catch(reject);
	});
};

export const moduleTagsMatch = async (module: AppModules): Promise<boolean> => {
	const url = `https://raw.githubusercontent.com/elysiae-project/components/refs/heads/main/components/${module}.json`;

	const installedTag = await getModuleVersion(module).catch((e) => {
		error(`moduleTagsMatch: ${e}`);
		return true;
	});
	const json = await getApiJson<ModuleData>(url).catch((e) => {
		error(`moduleTagsMatch: ${e}`);
		return true;
	});

	
	return false;
};
