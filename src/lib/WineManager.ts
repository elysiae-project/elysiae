import { appDataDir, join } from "@tauri-apps/api/path";
import { error, info } from "@tauri-apps/plugin-log";
import type {
	AppModules,
	ModuleData,
	WineComponent,
	WineComponentData,
	WineModule,
	WineSetupProgress,
} from "../types";
import { downloadFileWithProgress, executeLocalBinary, getApiJson } from "../util/AppFunctions";
import { getOption, setOption } from "../util/Settings";
import { exists, extractFile, remove, removeDir, rename } from "./Fs";

const components: ((
	onProgress: (event: WineSetupProgress) => void,
) => WineComponent)[] = [
	(onProgress) => ({
		componentName: "wine",
		extractTo: "wine",
		saveTo: "wine.tar.xz",
		postInstall: async () => {
			if (!(await exists(await join("wine", "drive_c")))) {
				await wineCommand("-i", "wineboot");
				await wineCommand("--wait", "wineserver");
				await wineCommand("-k", "wineserver");

				await installWineModules(onProgress);
			}
		},
	}),
	() => ({
		componentName: "dxvk",
		extractTo: "dxvk",
		saveTo: "dxvk.tar.gz",
		postInstall: async () => {
			const dirs = [
				{
					initialDirName: "x64",
					destDirName: "system32",
				},
				{
					initialDirName: "x32",
					destDirName: "syswow64",
				},
			] as const;

			const dlls = ["d3d9", "d3d10core", "d3d11", "dxgi"] as const;

			await Promise.all(
				dirs.map(async ({ initialDirName, destDirName }) => {
					const sourceDir = await join("dxvk", initialDirName);
					const destDir = await join("wine", "drive_c", "windows", destDirName);

					await Promise.all(
						dlls.map(async (dll) => {
							const sourceFile = await join(sourceDir, `${dll}.dll`);
							const destFile = await join(destDir, `${dll}.dll`);
							await rename(sourceFile, destFile);
						}),
					);
				}),
			);

			// Both x32 and x64 DLLs must be in place before the DLL registry keys can be added to the Wine registry
			await Promise.all(dlls.map(registerNewDLL));

			await removeDir("dxvk");
		},
	}),
	() => ({
		componentName: "jadeite",
		extractTo: "jadeite",
		saveTo: "jadeite.zip",
		postInstall: async () => {
			await executeLocalBinary("jadeite/block_analytics.sh");
		},
	}),
	/*(onProgress) => ({
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
  }),*/
];

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

/** Update all components in the wine install */
export const updateAllWineComponents = async (
	onProgress: (event: WineSetupProgress) => void,
): Promise<void> => {
	const modules: AppModules[] = ["wine", "dxvk", "jadeite"];
	for (let i = 0; i < modules.length; i++) {
		try {
			await updateWineComponent(modules[i], onProgress);
		} catch (e) {
			error(`updateAllWineComponents: ${e}`);
			return;
		}
	}
	info("Wine Component Download Complete");
	onProgress({ type: "wineSetupFinished" });
};

/**
 * Updates a specified wine component
 *
 * @param componentName A valid wine component name
 */
export const updateWineComponent = async (
	componentName: AppModules,
	onProgress: (event: WineSetupProgress) => void,
): Promise<void> => {
	const index = components.findIndex(
		(factory) => factory(onProgress).componentName === componentName,
	);
	const component = components[index](onProgress);

	try {
		info(`Installing/Updating ${component.componentName}`);
		const assetURL = `https://raw.githubusercontent.com/elysiae-project/components/refs/heads/main/components/${component.componentName}.json`;
		const assetResponse = await getApiJson<ModuleData[]>(assetURL);

		const json = assetResponse[0];

		onProgress({
			type: "wineSetupDownloading",
			component: component.componentName,
			downloaded_bytes: 0,
			total_bytes: 0,
		});

		await downloadFileWithProgress(
			json.download_url,
			component.saveTo,
			(progress, total) => {
				onProgress({
					type: "wineSetupDownloading",
					component: component.componentName,
					downloaded_bytes: progress,
					total_bytes: total,
				});
			},
		);

		onProgress({
			type: "wineSetupExtracting",
			component: component.componentName,
		});

		await extractFile(component.saveTo, component.extractTo);

		if (typeof component.postInstall !== "undefined") {
			onProgress({
				type: "wineSetupInstalling",
				component: component.componentName,
			});
			await component.postInstall();
		}

		// Update the wine module tracker
		await updateModuleTracker(component.componentName, json.tag);
	} catch (e) {
		error(`installWineComponent: ${e}`);
		return;
	}
	info(`Installation/Update of ${component.componentName} succeeded.`);
};

/**
 * Install additional Windows programs/libraries required for the programs that
 * Elysiae runs
 */
const installWineModules = async (
	onProgress: (event: WineSetupProgress) => void,
): Promise<void> => {
	await Promise.all(
		wineModules.map(async (module) => {
			info(`Installing ${module.name}`);

			const filename = module.downloadLink.split("/").pop() as string;

			onProgress({
				type: "wineSetupDownloading",
				component: module.name,
				downloaded_bytes: 0,
				total_bytes: 0,
			});

			await downloadFileWithProgress(
				module.downloadLink,
				filename,
				(progress, total) => {
					onProgress({
						type: "wineSetupDownloading",
						component: module.name,
						downloaded_bytes: progress,
						total_bytes: total,
					});
				},
			);

			if (module.moduleType === "exe") {
				await runExeWithWine(filename, "/install /quiet /norestart");
				await remove(filename);
			} else {
				if (module.moduleType === "dll64") {
					const sys32 = await join("wine", "drive_c", "windows", "system32");
					await rename(filename, `${sys32}/${filename}`);
				} else {
					const syswow = await join("wine", "drive_c", "windows", "syswow64");
					await rename(filename, `${syswow}/${filename}`);
				}
				await registerNewDLL(filename.split(".")[0]);
			}
		}),
	);
	info("Wine Module Download Complete");
};

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

const registerNewDLL = async (dllName: string): Promise<void> => {
	await wineCommand(
		`reg add 'HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides' /v ${dllName} /t REG_SZ /d native /f`,
	);
};

export const wineEnvAvailable = async (): Promise<boolean> => {
	const winePrefixPath = await winePrefix();
	const driveC = await join(winePrefixPath, "drive_c");

	return (
		(await exists(winePrefixPath)) &&
		(await exists(driveC)) &&
		(await exists("jadeite"))
	);
};

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
				if (typeof module === "undefined") {
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
		throw e;
	});
	if (installedTag !== null) {
		try {
			const json = await getApiJson<ModuleData[]>(url);
			return json[0].tag === installedTag;
		} catch (e: unknown) {
			error(`moduleTagsMatch: ${e}`);
		}
	}
	return false;
};
