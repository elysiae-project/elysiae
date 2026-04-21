import { useDownload } from "../../hooks/useDownload";
import { getGameName, formatNumber } from "../../util/AppFunctions";
import { pauseDownload, resumeDownload } from "../../lib/GameDownloader";
import Progressbar from "../Progressbar";
import { Pause, Play } from "lucide-preact";
import Button from "../Button";

export default function GameDownloadProgress() {
	const { state } = useDownload();
	const {
		isPaused,
		isDownloading,
		isAssembling,
		isVerifying,
		isFetchingManifest,
		isError,
		isFinished,
	} = state;

	const isActive =
		isDownloading ||
		isAssembling ||
		isVerifying ||
		isFetchingManifest ||
		isPaused;
	if (!isActive && !isError && !isFinished) return null;
	if (isFinished) return null;

	const downloadPct =
		state.downloadTotal > 0
			? (state.downloadedBytes / state.downloadTotal) * 100
			: 0;
	const assemblePct =
		state.totalFiles > 0 ? (state.assembledFiles / state.totalFiles) * 100 : 0;
	const speedMB = state.speedBps / 1024 ** 2;
	const eta = state.etaSeconds;
	const etaStr =
		eta > 0
			? eta >= 3600
				? `${Math.floor(eta / 3600)}h ${Math.floor((eta % 3600) / 60)}m`
				: `${Math.floor(eta / 60)}m ${Math.floor(eta % 60)}s`
			: "";

	const titleText = isPaused
		? "Download Paused"
		: isVerifying
			? "Verifying Files..."
			: isFetchingManifest
				? "Fetching Manifest..."
				: state.downloadingGame !== null
					? `Downloading ${getGameName(state.downloadingGame)}...`
					: "Downloading...";

	const canPause = isDownloading || isPaused;

	return (
		<div class="mr-10 flex h-auto w-full flex-col items-start justify-start gap-y-3 rounded-lg bg-black/50 px-4 py-5 align-bottom text-white">
			<div class="flex w-full flex-row items-center justify-between">
				<h1 class="-mt-2 mb-0.5">{titleText}</h1>
				{canPause && (
					<Button
						onClick={async () => {
							if (isPaused) {
								await resumeDownload();
							} else {
								await pauseDownload();
							}
						}}
						intent="secondary"
						iconButton>
						{isPaused ? (
							<Play className={"leading-0 -m-1"} />
						) : (
							<Pause className={"leading-0 -m-1"} />
						)}
					</Button>
				)}
			</div>
			{(isDownloading || isPaused) && state.downloadTotal > 0 && (
				<div class="flex min-w-full flex-col gap-y-1 text-left">
					<h2 class="ml-1 text-sm">
						Downloaded {(state.downloadedBytes / 1024 ** 3).toFixed(2)}GB of{" "}
						{(state.downloadTotal / 1024 ** 3).toFixed(2)}GB (
						{downloadPct.toFixed(2)}%)
						{speedMB > 0 ? ` - ${speedMB.toFixed(2)}MB/s` : ""}
						{etaStr ? ` - ETA: ${etaStr}` : ""}
					</h2>
					<Progressbar progress={downloadPct} />
				</div>
			)}
			{isAssembling && state.totalFiles > 0 && (
				<div class="flex min-w-full flex-col gap-y-1 text-left">
					<h2 class="ml-1 text-sm">
						Assembled {formatNumber(state.assembledFiles)} of{" "}
						{formatNumber(state.totalFiles)} Files ({assemblePct.toFixed(2)}%)
					</h2>
					<Progressbar progress={assemblePct} />
				</div>
			)}
			{isVerifying && (
				<div class="flex min-w-full flex-col gap-y-1 text-left">
					<h2 class="ml-1 text-sm">
						Verified {formatNumber(state.scannedFiles)} of{" "}
						{formatNumber(state.totalFiles)} files —{" "}
						{formatNumber(state.errorCount)} errors found
					</h2>
					<Progressbar
						progress={
							state.totalFiles > 0
								? (state.scannedFiles / state.totalFiles) * 100
								: 0
						}
					/>
				</div>
			)}
			{state.warningMessage && (
				<h2 class="ml-1 text-sm text-yellow-300">{state.warningMessage}</h2>
			)}
			{state.errorMessage && (
				<h2 class="ml-1 text-sm text-red-300">{state.errorMessage}</h2>
			)}
		</div>
	);
}
