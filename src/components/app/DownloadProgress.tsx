import { useDownload } from "../../hooks/useDownload";
import { useGame } from "../../hooks/useGame";
import { getGameName, formatNumber } from "../../util/AppFunctions";
import { pauseDownload, resumeDownload } from "../../lib/GameDownloader";
import Progressbar from "../Progressbar";
import { Pause, Play } from "lucide-preact";
import Button from "../Button";
import { useMemo } from "preact/hooks";

export default function GameDownloadProgress() {
	const { state } = useDownload();
	const { game } = useGame();
	const {
		isPaused,
		isDownloading,
		isAssembling,
		isVerifying,
		isFetchingManifest,
		isCalculatingDownloads,
		isError,
		isFinished,
	} = state;

	const isActive =
		isDownloading ||
		isAssembling ||
		isVerifying ||
		isFetchingManifest ||
		isCalculatingDownloads ||
		isPaused;
	if (!isActive && !isError && !isFinished) return null;
	if (isFinished) return null;

	const derived = useMemo(() => {
		const downloadPct =
			state.downloadTotal > 0
				? (state.downloadedBytes / state.downloadTotal) * 100
				: 0;
		const assemblePct =
			state.totalFiles > 0
				? (state.assembledFiles / state.totalFiles) * 100
				: 0;
		const speedMB = state.speedBps / 1024 ** 2;
		const eta = state.etaSeconds;
		const etaStr =
			eta > 0
				? eta >= 3600
					? `${Math.floor(eta / 3600)}h ${Math.floor((eta % 3600) / 60)}m`
					: `${Math.floor(eta / 60)}m ${Math.floor(eta % 60)}s`
				: "";
		const downloadedGB = (state.downloadedBytes / 1024 ** 3).toFixed(2);
		const totalGB = (state.downloadTotal / 1024 ** 3).toFixed(2);
		const verifyPct =
			state.totalFiles > 0 ? (state.scannedFiles / state.totalFiles) * 100 : 0;
		const calcPct =
			state.totalFiles > 0 ? (state.checkedFiles / state.totalFiles) * 100 : 0;
		return {
			downloadPct,
			assemblePct,
			speedMB,
			etaStr,
			downloadedGB,
			totalGB,
			verifyPct,
			calcPct,
		};
	}, [
		state.downloadedBytes,
		state.downloadTotal,
		state.assembledFiles,
		state.totalFiles,
		state.speedBps,
		state.etaSeconds,
		state.scannedFiles,
		state.checkedFiles,
	]);

	const titleText = isPaused
		? "Download Paused"
		: isVerifying
			? "Verifying Files..."
			: isCalculatingDownloads
				? "Calculating File Downloads..."
				: isFetchingManifest
					? "Fetching Manifest..."
					: state.downloadingGame !== null
						? `Downloading ${getGameName(state.downloadingGame)}...`
						: "Downloading...";

	const canPause = isDownloading || isPaused;

	return (
		<div class="mr-10 flex h-auto w-[65%] flex-col items-start justify-start gap-y-3 rounded-lg bg-black/50 px-4 py-5 align-bottom">
			<div class="flex w-full flex-row items-center justify-between">
				<h1 class="-mt-2 mb-0.5 text-white">{titleText}</h1>
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
			{isCalculatingDownloads && state.totalFiles > 0 && (
				<div class="flex min-w-full flex-col gap-y-1 text-left">
					<h2 class="ml-1 text-sm text-white">
						Checked {formatNumber(state.checkedFiles)} of{" "}
						{formatNumber(state.totalFiles)} Files ({derived.calcPct.toFixed(2)}
						%)
					</h2>
					<Progressbar progress={derived.calcPct} game={game} />
				</div>
			)}
			{(isDownloading || isPaused) && state.downloadTotal > 0 && (
				<div class="flex min-w-full flex-col gap-y-1 text-left">
					<h2 class="ml-1 text-sm text-white">
						Downloaded {derived.downloadedGB}GB of {derived.totalGB}GB (
						{derived.downloadPct.toFixed(2)}%)
						{derived.speedMB > 0 ? ` - ${derived.speedMB.toFixed(2)}MB/s` : ""}
						{derived.etaStr ? ` - ETA: ${derived.etaStr}` : ""}
					</h2>
					<Progressbar progress={derived.downloadPct} game={game} />
				</div>
			)}
			{isAssembling && state.totalFiles > 0 && (
				<div class="flex min-w-full flex-col gap-y-1 text-left">
					<h2 class="ml-1 text-sm text-white">
						Assembled {formatNumber(state.assembledFiles)} of{" "}
						{formatNumber(state.totalFiles)} Files (
						{derived.assemblePct.toFixed(2)}%)
					</h2>
					<Progressbar progress={derived.assemblePct} game={game} />
				</div>
			)}
			{isVerifying && (
				<div class="flex min-w-full flex-col gap-y-1 text-left">
					<h2 class="ml-1 text-sm text-white">
						Verified {formatNumber(state.scannedFiles)} of{" "}
						{formatNumber(state.totalFiles)} files —{" "}
						{formatNumber(state.errorCount)} errors found
					</h2>
					<Progressbar progress={derived.verifyPct} game={game} />
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
