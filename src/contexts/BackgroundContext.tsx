import { invoke } from "@tauri-apps/api/core";
import { join } from "@tauri-apps/api/path";
import { fetch } from "@tauri-apps/plugin-http";
import { type ComponentChildren, createContext } from "preact";
import { useEffect, useState } from "preact/hooks";
import { useGame } from "../hooks/useGame";
import { exists, getDirFileNames, mkdir, remove } from "../lib/Fs";
import { getOption, setOption } from "../lib/Settings";
import { variantToGameCode } from "../lib/VariantConverter";
import { downloadFileNoProgress } from "../lib/Web";
import type { CachedBackgrounds, LauncherBackgroundRawData } from "../types";

const ASSETS_ENDPOINT = "https://assets.elysiae.app";
type BackgroundItems = {
	path: string;
	name: string;
	url: string;
};

interface BackgroundContextType {
	cachedBackgrounds: CachedBackgrounds | null;
	backgroundSrc: string | null;
	backgroundIsVideo: boolean;
}

export const BackgroundContext = createContext<BackgroundContextType>({
	cachedBackgrounds: null,
	backgroundSrc: null,
	backgroundIsVideo: false,
});

export const BackgroundProvider = ({
	children,
}: {
	children: ComponentChildren;
}) => {
	const [cachedBackgroundData, setCachedBackgroundData] =
		useState<CachedBackgrounds | null>(null);

	const [currentBackgroundSrc, setCurrentBackgroundSrc] = useState<
		string | null
	>(null);

	const [currentBackgroundIsVideo, setCurrentBackgroundIsVideo] =
		useState(false);

	const { game } = useGame();

	useEffect(() => {
		(async () => {
			const cachedBackgrounds =
				await getOption<CachedBackgrounds>("cachedBackgrounds");
			setCachedBackgroundData(cachedBackgrounds ?? null);
		})();
	}, []);

	useEffect(() => {
		if (!cachedBackgroundData) return;
		if (!cachedBackgroundData[game] || cachedBackgroundData[game].length === 0)
			return;

		const paths = cachedBackgroundData[game];
		const mp4Index = paths.findIndex((i) => i.endsWith(".mp4"));
		const index = mp4Index === -1 ? 0 : mp4Index;
		const relativePath = paths[index];
		const isVideo = mp4Index !== -1;

		(async () => {
			const port = await invoke<number>("media_server_port");
			const encoded = relativePath.split("/").map(encodeURIComponent).join("/");
			setCurrentBackgroundSrc(`http://127.0.0.1:${port}/${encoded}`);
			setCurrentBackgroundIsVideo(isVideo);
		})();
	}, [cachedBackgroundData, game]);

	useEffect(() => {
		(async () => {
			const gameCode = variantToGameCode[game];
			const backgroundDir = await join("backgrounds", gameCode);
			if (!(await exists(backgroundDir))) {
				await mkdir(backgroundDir);
			}

			const response = await fetch(`${ASSETS_ENDPOINT}/launcherAssets.json`);
			const apiData: LauncherBackgroundRawData = await response.json();

			const localDirFileNames = await getDirFileNames(backgroundDir);

			const localItems: BackgroundItems[] = [];
			const webItems: BackgroundItems[] = [];

			await Promise.all(
				localDirFileNames.map(async (fileName) => {
					const path = await join(backgroundDir, fileName);
					localItems.push({
						path: path,
						name: fileName,
						url: "",
					});
				}),
			);

			const assets = apiData[variantToGameCode[game]];
			await Promise.all(
				assets.map(async (asset) => {
					const currentVideo = asset.video;
					const currentImg = asset.image;

					await Promise.all(
						[currentImg, currentVideo].map(async (item) => {
							if (item) {
								const fileName = item.split("/").pop()?.trim() as string;
								const localPath = await join(backgroundDir, fileName);
								const url = `${ASSETS_ENDPOINT}/${item}`;
								webItems.push({
									path: localPath,
									name: fileName,
									url: url,
								});
							}
						}),
					);
				}),
			);

			const toDelete = localItems.filter(
				(i) => !webItems.some((w) => w.name === i.name),
			);
			const toDownload = webItems.filter(
				(i) => !localItems.some((l) => l.name === i.name),
			);

			if (toDownload.length !== 0) {
				await Promise.all(
					toDownload.map(async (file) => {
						try {
							await downloadFileNoProgress(file.url, file.path);
						} catch {
							try {
								await remove(file.path);
							} catch {}
						}
					}),
				);

				await Promise.all(
					toDelete.map(async (file) => {
						await remove(file.path);
					}),
				);
			}

			const syncedFileNames = await getDirFileNames(backgroundDir);
			const syncedPaths: string[] = [];
			await Promise.all(
				syncedFileNames.map(async (fileName) => {
					const path = await join(backgroundDir, fileName);
					syncedPaths.push(path);
				}),
			);

			const newBackgroundData: CachedBackgrounds = {
				...((cachedBackgroundData ?? {}) as Record<number, string[]>),
				[game]: syncedPaths,
			} as CachedBackgrounds;
			setCachedBackgroundData(newBackgroundData);

			await setOption<CachedBackgrounds>(
				"cachedBackgrounds",
				newBackgroundData,
			);
		})();
	}, [game]);

	return (
		<BackgroundContext.Provider
			value={{
				cachedBackgrounds: cachedBackgroundData,
				backgroundSrc: currentBackgroundSrc,
				backgroundIsVideo: currentBackgroundIsVideo,
			}}
		>
			{children}
		</BackgroundContext.Provider>
	);
};
