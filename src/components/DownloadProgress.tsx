import { useEffect, useState } from "preact/hooks";
import Progressbar from "./Progressbar";
import { downloadEvent, downloads } from "../util/DownloadManager";

export default function DownloadProgress() {
	let [isDownloading, setIsDownloading] = useState<boolean>(false);
	let [downloaded, setDownloaded] = useState<number>(0);
	let [total, setTotal] = useState<number>(0);
	let [progress, setProgress] = useState<number>(0);

	useEffect(() => {
		const callback = () => {
			if (downloads.size == 0 && !isDownloading) {
				setIsDownloading(true);
			} else if (downloads.size > 0 && isDownloading) {
				if (!isDownloading) {
					setIsDownloading(false);
				}
				const downloadSize = downloads.size;
				let rawDownloaded: number = 0;
				let rawTotal: number = 0;
				downloads.forEach((download) => {
					rawDownloaded += download.downloaded;
					rawTotal += download.total;
				});

				setDownloaded(rawDownloaded / downloadSize);
				setTotal(rawTotal / downloadSize);
				setProgress(downloaded / total);
			}
		};
		downloadEvent.addEventListener("downloadChanged", callback);

		return () => {
			downloadEvent.removeEventListener("downloadChanged", callback);
		};
	}, []);

	if (!isDownloading) return <></>;
	return (
		<div class="flex flex-row gap-y-1.5">
			<p class="text-lg">
				Downloaded {downloaded / 1024}GB of {total / 1024}GB (
				{progress.toFixed(2)}%)
			</p>
			<Progressbar progress={progress} />
		</div>
	);
}
