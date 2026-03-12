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
				if(rawDownloaded === 0 && rawTotal === 0) {
					setIsDownloading(false);
				} 

				setDownloaded(rawDownloaded);
				setTotal(rawTotal);
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
		<div class="flex flex-col gap-y-1.5 bg-black mr-5 px-2 py-3 rounded-lg min-w-110 text-center">
			<p class="text-lg text-white">
				Downloaded {(downloaded / 1024).toFixed(2)}Gb of {(total / 1024).toFixed(2)}Gb (
				{(progress).toFixed(2)}%)
			</p>
			<Progressbar progress={progress} />
		</div>
	);
}
