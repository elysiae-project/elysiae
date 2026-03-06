import { useEffect, useState } from "preact/hooks";
import Progressbar from "./Progressbar";
import { downloadEvent, downloads } from "../util/DownloadManager";
import { info } from "@tauri-apps/plugin-log";

export default function DownloadProgress() {
	let [isDownloading, setIsDownloading] = useState<boolean>(false);
	let [downloaded, setDownloaded] = useState<number>(0);
	let [total, setTotal] = useState<number>(0);
	let [progress, setProgress] = useState<number>(0);

	useEffect(() => {
		const callback = () => {
			if (downloads.size > 0 && !isDownloading) {
				setIsDownloading(true);
			} else if (downloads.size == 0 && isDownloading) {
				setIsDownloading(false);
			}

			if (isDownloading) {
                //info('invoked');
				const downloadSize = downloads.size;
				let rawDownloaded: number = 0;
				let rawTotal: number = 0;
				downloads.forEach((download) => {
					rawDownloaded += download.downloaded;
					rawTotal += download.total;
				});

				setDownloaded(rawDownloaded / downloadSize);
				setTotal(rawTotal / downloadSize);
				setProgress((rawDownloaded / downloadSize) / (rawTotal / downloadSize) * 100);
			}
		};
		downloadEvent.addEventListener("downloadChanged", callback);

		return () => {
			downloadEvent.removeEventListener("downloadChanged", callback);
		};
	}, []);

	if (!isDownloading) return <></>;
	return (
		<div class="flex flex-col gap-y-1.5 bg-[#000000aa] p-2 mx-5 rounded-lg min-w-80">
			<p class="text-lg">
				Downloaded {(downloaded / 1024).toFixed(2)}GB of {(total / 1024).toFixed(2)}GB (
				{(progress).toFixed(2)}%)
			</p>
			<Progressbar progress={progress} />
		</div>
	);
}
