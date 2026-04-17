import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "preact/hooks";
import { SophonProgress } from "../../types";
import Progressbar from "../Progressbar";
import { formatNumber } from "../../util/AppFunctions";

export default function GameDownloadProgress() {
	let [downloadedBytes, setDownloadedBytes] = useState<number>(0);
	let [downloadTotal, setDownloadTotal] = useState<number>(0);

	let [assembledFiles, setAssembledFiles] = useState<number>(0);
	let [totalFiles, setTotalFiles] = useState<number>(0);

	let [isDownloading, setIsDownloading] = useState<boolean>(false);

	useEffect(() => {
		const unlisten = listen("sophon://progress", (event) => {
			const payload = event.payload as SophonProgress;
			switch (payload.type) {
				case "downloading":
					if (!isDownloading) setIsDownloading(true);
					if (downloadTotal === 0) setDownloadTotal(payload.total_bytes);

					setDownloadedBytes(payload.downloaded_bytes);
					break;
				case "assembling":
					if (!isDownloading) setIsDownloading(true);
					if (totalFiles === 0) setTotalFiles(payload.total_files);

					setAssembledFiles(payload.assembled_files);
					break;
				case "finished":
					setIsDownloading(false);
					[
						setDownloadedBytes,
						setDownloadTotal,
						setAssembledFiles,
						setTotalFiles,
					].forEach((set) => {
						set(0);
					});
					break;
			}
		});
		return async () => {
			(await unlisten)();
		};
	}, []);

	if (!isDownloading) return null;
	return (
		<div class="flex flex-col w-full h-auto gap-y-3 justify-start items-start align-bottom text-white bg-black/50 rounded-lg mr-10 px-4 py-5">
			<h1 class="-mt-2 mb-0.5">Downloading GAME...</h1>{" "}
			{/** TODO: Replace with Game ID when sophon downloader gets refactored to also emit which game is being downloaded */}
			<div class="flex flex-col min-w-full text-left gap-y-1">
				<h2 class="text-sm ml-1">
					Downloaded {(downloadedBytes / 1024 ** 3).toFixed(2)}GB of{" "}
					{(downloadTotal / 1024 ** 3).toFixed(2)}GB (
					{((downloadedBytes / downloadTotal) * 100).toFixed(2)}%)
				</h2>
				<Progressbar progress={(downloadedBytes / downloadTotal) * 100} />
			</div>
			<div class="flex min-w-full flex-col text-left gap-y-1">
				<h2 class="text-sm ml-1">
					Assembled {formatNumber(assembledFiles)} of {formatNumber(totalFiles)}{" "}
					{/** FIXME: There is an inconsistency here where only decimal places will be used for percentage rather than what the locale wants for decimals */}
					Files ({((assembledFiles / totalFiles) * 100).toFixed(2)}%)
				</h2>
				<Progressbar progress={(assembledFiles / totalFiles) * 100} />
			</div>
		</div>
	);
}
