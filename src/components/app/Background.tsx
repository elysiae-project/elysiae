import { convertFileSrc } from "@tauri-apps/api/core";
import { join } from "@tauri-apps/api/path";
import { fetch } from "@tauri-apps/plugin-http";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "preact/hooks";
import { useApi } from "../../hooks/useApi";
import { useGame } from "../../hooks/useGame";
import { getDirFileNames, remove } from "../../lib/Fs";
import { getOption, setOption } from "../../lib/Settings";
import { variantToGameCode } from "../../lib/VariantConverter";
import { downloadFileNoProgress } from "../../lib/Web";
import type { CachedBackgrounds, LauncherBackgroundRawData } from "../../types";

type BackgroundItems = {
	path: string;
	name: string;
};

const BackgroundMedia = ({
	webSrc,
	isVideo,
}: {
	webSrc: string | null;
	isVideo: boolean;
}) => {
	if (!webSrc) return null;

	return isVideo ? (
		<motion.video
			class="background"
			initial={{ opacity: 0 }}
			animate={{ opacity: 1 }}
			exit={{ opacity: 0 }}
			transition={{ duration: 0.25, ease: "easeInOut" }}
			src={webSrc as string}
			autoplay
			loop
			muted
			playsInline
		/>
	) : (
		<motion.img
			class="background"
			initial={{ opacity: 0 }}
			animate={{ opacity: 1 }}
			exit={{ opacity: 0 }}
			transition={{ duration: 0.25, ease: "easeInOut" }}
			src={webSrc as string}
			alt=""
		/>
	);
};

export const Background = () => {
	const { game } = useGame();
	const { graphics, webBackgrounds } = useApi();

	const [bgPath, setBgPath] = useState<string | null>(null);

	useEffect(() => {
		(async () => {
			// First, set the background path to the cached file
			const gameCode = variantToGameCode[game];
			const backgroundDir = await join("backgrounds", gameCode);

			const cachedBackgrounds =
				await getOption<CachedBackgrounds>("cachedBackgrounds");

			// Get the first video file found. if none exist, just default to index 0
			if (cachedBackgrounds[game] && cachedBackgrounds[game].length > 0) {
				const initialIndex =
					cachedBackgrounds[game].findIndex((i) => i.endsWith(".mp4")) ?? 0;
				setBgPath(convertFileSrc(cachedBackgrounds[game][initialIndex]));
			}

			// Now, update the cached assets
			const apiData: LauncherBackgroundRawData = await (
				await fetch("https://assets.elysiae.app/launcherAssets.json")
			).json();

			const gameCodeAssets = `https://assets.elysiae.app/data/${gameCode}`;
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

				cachedBackgrounds[game] = newBackgroundPaths;
				await setOption<CachedBackgrounds>(
					"cachedBackgrounds",
					cachedBackgrounds,
				);
			}
		})();
	}, [game]);

	if (!bgPath) return null;

	if (!graphics || !webBackgrounds) return null;
	const { backgroundImage, backgroundVideo } = webBackgrounds[game];
	const { backgroundVideoOverlay } = graphics[game];

	const isVideo = backgroundVideo !== null && backgroundVideo !== "";

	return (
		<div class="absolute inset-0 overflow-hidden">
			<AnimatePresence mode="wait">
				<BackgroundMedia
					key={`${game}-bg`}
					webSrc={isVideo ? backgroundVideo : backgroundImage}
					isVideo={isVideo}
				/>
			</AnimatePresence>
			<AnimatePresence mode="wait">
				<BackgroundMedia
					key={`${game}-overlay`}
					webSrc={backgroundVideoOverlay}
					isVideo={false}
				/>
			</AnimatePresence>
		</div>
	);
};

export default Background;
