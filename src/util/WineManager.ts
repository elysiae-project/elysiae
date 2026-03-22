import { appDataDir, join } from "@tauri-apps/api/path";
import { ComponentData, WineComponent, WineModule } from "../types";
import { exists, remove, removeDir, rename } from "./Fs";
import { fetch } from "@tauri-apps/plugin-http";
import { downloadFile } from "./WebUtils";
import { extractFile } from "./FileUtils";
import { error, info } from "@tauri-apps/plugin-log";
import { executeLocalCommand, executeShellCommand } from "./AppFunctions";

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
			removeDir("dxvk");
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
		},
	},
] as const;

// Components that do not need to be updated (i.e. Visual C++ Redistributable)
const wineModules: WineModule[] = [
	{
		name: "vcrun2026",
		downloadLink: "https://aka.ms/vc14/vc_redist.x64.exe",
		moduleType: "exe",
	},
] as const;

export const updateWineComponents = async () => {
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
};

const installWineModules = async () => {
	await Promise.all(
		wineModules.map(async (module) => {
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
};

export const wineCommand = async (
	args: string,
	binary: "wine" | "wineboot" | "wineserver" = "wine",
) => {
	const prefix = await winePrefix();
	await executeLocalCommand(`wine/bin/${binary}`, args, {
		WINEPREFIX: prefix,
		WINEFSYNC: "1",
	});
};

export const runExeWithWine = async (path: string, args?: string) => {
	const appData = await appDataDir();
	const fullPath = await join(appData, path);

	await wineCommand(`${fullPath} ${typeof args !== "undefined" ? args : ""}`);
};

export const registerNewDLL = async (dllName: string) => {
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
