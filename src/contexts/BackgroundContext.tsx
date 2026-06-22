import { join } from "@tauri-apps/api/path";
import { fetch } from "@tauri-apps/plugin-http";
import { error, info } from "@tauri-apps/plugin-log";
import { type ComponentChildren, createContext } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { useGame } from "../hooks/useGame";
import { exists, getDirFileNames, mkdir, readFile, remove } from "../lib/Fs";
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

const MIME_TYPES: Record<string, string> = {
	mp4: "video/mp4",
	webm: "video/webm",
	webp: "image/webp",
	png: "image/png",
	jpg: "image/jpeg",
	jpeg: "image/jpeg",
	gif: "image/gif",
	svg: "image/svg+xml",
};

function getMimeType(path: string): string {
	const ext = path.split(".").pop()?.toLowerCase() ?? "";
	return MIME_TYPES[ext] ?? "application/octet-stream";
}

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

	const blobUrlRef = useRef<string | null>(null);
	const { game } = useGame();

	useEffect(() => {
		(async () => {
			try {
				const cachedBackgrounds =
					await getOption<CachedBackgrounds>("cachedBackgrounds");
				setCachedBackgroundData(cachedBackgrounds ?? null);
			} catch (e) {
				error(`BackgroundProvider: Failed to load cached backgrounds: ${e}`);
			}
		})();
	}, []);

	useEffect(() => {
		return () => {
			if (blobUrlRef.current) {
				URL.revokeObjectURL(blobUrlRef.current);
				blobUrlRef.current = null;
			}
		};
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
			if (blobUrlRef.current) {
				URL.revokeObjectURL(blobUrlRef.current);
				blobUrlRef.current = null;
			}

			try {
				const fileData = await readFile(relativePath);
				const mimeType = getMimeType(relativePath);
				const blob = new Blob([fileData], { type: mimeType });
				const blobUrl = URL.createObjectURL(blob);
				blobUrlRef.current = blobUrl;
				setCurrentBackgroundSrc(blobUrl);
				setCurrentBackgroundIsVideo(isVideo);
			} catch (e) {
				error(`BackgroundProvider: Failed to load background as blob: ${e}`);
			}
		})();
	}, [cachedBackgroundData, game]);

	useEffect(() => {
		(async () => {
			try {
				const gameCode = variantToGameCode[game];
				const backgroundDir = await join("backgrounds", gameCode);
				if (!(await exists(backgroundDir))) {
					await mkdir(backgroundDir);
				}

				info(`BackgroundProvider: Fetching assets for ${gameCode}`);
				const response = await fetch(`${ASSETS_ENDPOINT}/launcherAssets.json`);
				if (!response.ok) {
					throw new Error(`HTTP ${response.status}`);
				}
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
				if (!assets) {
					info(`BackgroundProvider: No assets found for ${gameCode}`);
					return;
				}

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
			} catch (e) {
				error(`BackgroundProvider: Failed to sync backgrounds: ${e}`);
			}
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
