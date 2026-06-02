import { convertFileSrc } from "@tauri-apps/api/core";
import { join } from "@tauri-apps/api/path";
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
};

interface BackgroundContextType {
	cachedBackgrounds: CachedBackgrounds | null;
	currentBackground: string | null;
}

export const BackgroundContext = createContext<BackgroundContextType>({
	cachedBackgrounds: null,
	currentBackground: null,
});

export const BackgroundProvider = ({
	children,
}: {
	children: ComponentChildren;
}) => {
	const [cachedBackgroundData, setCachedBackgroundData] =
		useState<CachedBackgrounds | null>(null);

	const [currentBackgroundPath, setCurrentBackgroundPath] = useState<
		string | null
	>(null);

	const { game } = useGame();

	useEffect(() => {
		// Initial cached background data load
		(async () => {
			const cachedBackgrounds =
				await getOption<CachedBackgrounds>("cachedBackgrounds");
			setCachedBackgroundData(cachedBackgrounds);
		})();
	}, []);

	useEffect(() => {
		(async () => {
			const gameCode = variantToGameCode[game];
			const backgroundDir = await join("backgrounds", gameCode);

			if (cachedBackgroundData) {
				if (
					cachedBackgroundData[game] &&
					cachedBackgroundData[game].length > 0
				) {
					const index =
						cachedBackgroundData[game].findIndex((i) => i.endsWith(".mp4")) ??
						0;
					const fullPath = await join(
						backgroundDir,
						cachedBackgroundData[game][index],
					);
					setCurrentBackgroundPath(convertFileSrc(fullPath));
					console.log(`Background Path: ${currentBackgroundPath}`);
				}
			}
		})();
	}, [cachedBackgroundData]);

	useEffect(() => {
		(async () => {
			const gameCode = variantToGameCode[game];
			const backgroundDir = await join("backgrounds", gameCode);
			if (!(await exists(backgroundDir))) {
				await mkdir(backgroundDir);
			}

			// Now, update the cached assets
			const apiData: LauncherBackgroundRawData = await (
				await fetch(`${ASSETS_ENDPOINT}/launcherAssets.json`)
			).json();

			const gameCodeAssets = `${ASSETS_ENDPOINT}/data/${gameCode}`;
			const localDirFileNames = await getDirFileNames(backgroundDir);

			// This type simplifies the code just a bit by storing the file name and the file path in one variable
			const localItems: BackgroundItems[] = [];
			const webItems: BackgroundItems[] = [];

			await Promise.all(
				localDirFileNames.map(async (fileName) => {
					const path = await join(backgroundDir, fileName);
					localItems.push({
						path: path,
						name: fileName,
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
								webItems.push({
									path: localPath,
									name: fileName,
								});
							}
						}),
					);
				}),
			);

			const toDelete = localItems.filter((i) => webItems.indexOf(i) < 0);
			const toDownload = webItems.filter((i) => localItems.indexOf(i) < 0);

			if (toDownload.length !== 0) {
				await Promise.all(
					toDownload.map(async (file) => {
						const url = `${gameCodeAssets}/${file.name}`;
						await downloadFileNoProgress(url, file.path);
					}),
				);

				await Promise.all(
					toDelete.map(async (file) => {
						await remove(file.path);
					}),
				);
				const newFileNames = getDirFileNames(backgroundDir);
				const newBackgroundPaths: string[] = [];
				await Promise.all(
					(await newFileNames).map(async (fileName) => {
						const path = await join(backgroundDir, fileName);
						newBackgroundPaths.push(path);
					}),
				);

				if (cachedBackgroundData) {
					const newBackgroundData = Object.assign(cachedBackgroundData);
					newBackgroundData[game] = newBackgroundPaths;
					setCachedBackgroundData(newBackgroundData);

					await setOption<CachedBackgrounds>(
						"cachedBackgrounds",
						newBackgroundData,
					);
				}
			}
		})();
	}, [game]);

	return (
		<BackgroundContext.Provider
			value={{
				cachedBackgrounds: cachedBackgroundData,
				currentBackground: currentBackgroundPath,
			}}
		>
			{children}
		</BackgroundContext.Provider>
	);
};
