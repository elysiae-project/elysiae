import { appDataDir, join } from "@tauri-apps/api/path";
import {
	ComponentData,
	WineComponent,
	WineModule,
	WineSetupProgress,
} from "../types";
import { exists, extractFile, remove, removeDir, rename } from "./Fs";
import { fetch } from "@tauri-apps/plugin-http";
import { downloadFileWithProgress } from "../util/WebUtils";
import { error, info } from "@tauri-apps/plugin-log";
import { executeLocalBinary, executeShellCommand } from "../util/AppFunctions";

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
	() => ({
		componentName: "vkd3d",
		extractTo: "vkd3d-temp",
		saveTo: "vkd3d.tar.zst",
		postInstall: async () => {
			await executeLocalBinary("vkd3d-temp/setup_vkd3d_proton.sh", "install", {
				WINEPREFIX: await winePrefix(),
			});

			await removeDir("vkd3d");
		},
	}),
];

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

export const updateWineComponents = async (
	onProgress: (event: WineSetupProgress) => void,
): Promise<void> => {
	info(`${await appDataDir()}`);
	for (const componentFactory of components) {
		const component = componentFactory(onProgress);
		try {
			info(`Installing ${component.componentName}`);

			const assetURL = `https://raw.githubusercontent.com/elysiae-project/components/refs/heads/main/components/${component.componentName}.json`;
			const response = await fetch(assetURL);
			if (response.status === 200) {
				const json: ComponentData[] = await response.json();

				onProgress({
					type: "wineSetupDownloading",
					component: component.componentName,
					downloaded_bytes: 0,
					total_bytes: 0,
				});

				await downloadFileWithProgress(
					json[0].download_url,
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
			} else {
				throw new Error("Endpoint returned non-OK response code");
			}
		} catch (e) {
			error(`updateWineComponents ${e}`);
		}
	}
	info("Wine Component Download Complete");
	onProgress({ type: "wineSetupFinished" });
};

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
		WINEFSYNC: "1",
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
	const winePath = await winePrefix();
	const driveC = await join(winePath, "drive_c");

	if ((await exists(winePath)) && (await exists(driveC))) {
		return true;
	}
	return false;
};

export const winePrefix = async (): Promise<string> => {
	return new Promise((resolve) => {
		appDataDir().then((appData) => {
			join(appData, "wine").then((res) => {
				resolve(res as string);
			});
		});
	});
};
