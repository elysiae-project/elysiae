import { appDataDir, join } from "@tauri-apps/api/path";
import { error, info } from "@tauri-apps/plugin-log";
import type {
	AppModules,
	ModuleData,
	ProtonComponent,
	ProtonComponentData,
	ProtonSetupProgress,
} from "../types";
import { exists, extractFile, mkdir } from "./Fs";
import { getOption, setOption } from "./Settings";
import { executeLocalBinary } from "./ShellCommands";
import { downloadFile, getApiJson } from "./Web";

const components: ((
	onProgress: (event: ProtonSetupProgress) => void,
) => ProtonComponent)[] = [
	() => ({
		componentName: "proton",
		extractTo: "proton",
		saveTo: "proton-ge.tar.gz",
		postInstall: async () => {
			await mkdir("proton-data");
		},
	}),
	() => ({
		// This will likely not be needed in the coming months. TODO: Remove when tests show that it isn't needed
		componentName: "jadeite",
		extractTo: "jadeite",
		saveTo: "jadeite.zip",
		postInstall: async () => {
			await executeLocalBinary("jadeite/block_analytics.sh");
		},
	}),
];

/**
 * Update all components in the Proton install
 */
export const updateAllProtonComponents = async (
	onProgress: (event: ProtonSetupProgress) => void,
): Promise<void> => {
	const modules: AppModules[] = ["proton", "jadeite"];
	for (let i = 0; i < modules.length; i++) {
		try {
			await updateProtonComponent(modules[i], onProgress);
		} catch (e) {
			error(`updateAllProtonComponents: ${e}`);
			return;
		}
	}
	info("Proton Component Download Complete");
	onProgress({ type: "protonSetupFinished" });
};

/**
 * Updates a specified Proton component
 *
 * @param componentName A valid Proton component name
 */
export const updateProtonComponent = async (
	componentName: AppModules,
	onProgress: (event: ProtonSetupProgress) => void,
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
			type: "protonSetupDownloading",
			component: component.componentName,
			downloaded_bytes: 0,
			total_bytes: 0,
		});

		await downloadFile(
			json.download_url,
			component.saveTo,
			(progress, total) => {
				onProgress({
					type: "protonSetupDownloading",
					component: component.componentName,
					downloaded_bytes: progress,
					total_bytes: total,
				});
			},
		);

		onProgress({
			type: "protonSetupExtracting",
			component: component.componentName,
		});

		await extractFile(component.saveTo, component.extractTo);

		if (typeof component.postInstall !== "undefined") {
			onProgress({
				type: "protonSetupInstalling",
				component: component.componentName,
			});
			await component.postInstall();
		}

		// Update the Proton module tracker
		await updateModuleTracker(component.componentName, json.tag);
	} catch (e) {
		error(`installProtonComponent: ${e}`);
		return;
	}
	info(`Installation/Update of ${component.componentName} succeeded.`);
};

export const protonCommand = async (args: string): Promise<void> => {
	const dataDir = await protonData();
	await executeLocalBinary(`proton/proton`, `run ${args}`, {
		STEAM_COMPAT_DATA_PATH: dataDir,
		STEAM_COMPAT_CLIENT_INSTALL_PATH: "", // This does nothing, but is required to be here for proton to run
	});
};

export const protonExec = async (
	path: string,
	args?: string,
): Promise<void> => {
	const appData = await appDataDir();
	const fullPath = await join(appData, path);

	await protonCommand(`${fullPath} ${typeof args !== "undefined" ? args : ""}`);
};

export const protonJadeiteExec = async (path: string): Promise<void> => {
	const appData = await appDataDir();
	const fullJadeitePath = await join(appData, "jadeite", "jadeite.exe");
	const fullExePath = await join(appData, path);
	await protonCommand(`${fullJadeitePath} ${fullExePath}`);
};

export const protonAvailable = async (): Promise<boolean> => {
	const protonInstallPath = await protonDir();
	const protonDataDir = await protonData();

	return (
		(await exists(protonInstallPath)) &&
		(await exists(protonDataDir)) &&
		(await exists("jadeite"))
	);
};

export const protonDir = async (): Promise<string> => {
	return await join(await appDataDir(), "proton");
};

const protonData = async (): Promise<string> => {
	return await join(await appDataDir(), "proton-data");
};

export const updateModuleTracker = async (
	module: AppModules,
	newVersion: string,
) => {
	const current =
		(await getOption<ProtonComponentData>("installedComponents")) ??
		({} as ProtonComponentData);
	current[module] = newVersion;
	await setOption("installedComponents", current);
};

export const getModuleVersion = async (
	module: AppModules | undefined = undefined,
): Promise<ProtonComponentData | string | null> => {
	const data =
		(await getOption<ProtonComponentData>("installedComponents")) ??
		({} as ProtonComponentData);
	if (typeof module === "undefined") {
		return data;
	}
	return data[module as AppModules] ?? null;
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
