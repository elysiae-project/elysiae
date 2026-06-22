import { Pause, Play } from "lucide-preact";
import { useMemo } from "preact/hooks";
import { useDownload } from "../../hooks/useDownload";
import { useGame } from "../../hooks/useGame";
import { pauseDownload, resumeDownload } from "../../lib/GameDownloader";
import { variantToGameName } from "../../lib/VariantConverter";
import Button from "../Button";
import Progressbar from "../Progressbar";

const formatNumber = (num: number): string => {
	try {
		return new Intl.NumberFormat(navigator.language).format(num);
	} catch {
		return new Intl.NumberFormat("en-US").format(num);
	}
};

export const DownloadProgress = () => {
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
		isSettingUpProton,
	} = state;

	const isActive =
		isDownloading ||
		isAssembling ||
		isVerifying ||
		isFetchingManifest ||
		isCalculatingDownloads ||
		isPaused ||
		isSettingUpProton;
	if (!isActive && !isError && !isFinished) return null;
	if (isFinished) return null;

	const protonSetupPct = useMemo(() => {
		if (!isSettingUpProton || state.protonSetupDownloadTotal <= 0) return 0;
		if (state.protonSetupPhase !== "downloading") return 100;
		return (
			(state.protonSetupDownloadedBytes / state.protonSetupDownloadTotal) * 100
		);
	}, [
		isSettingUpProton,
		state.protonSetupPhase,
		state.protonSetupDownloadedBytes,
		state.protonSetupDownloadTotal,
	]);

	const protonPhaseLabel = useMemo(() => {
		switch (state.protonSetupPhase) {
			case "downloading":
				return "Downloading";
			case "extracting":
				return "Extracting";
			case "installing":
				return "Installing";
			default:
				return "";
		}
	}, [state.protonSetupPhase]);

	const protonDownloadedMB = (
		state.protonSetupDownloadedBytes /
		1024 ** 2
	).toFixed(1);
	const protonTotalMB = (state.protonSetupDownloadTotal / 1024 ** 2).toFixed(1);

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
				? `${String(Math.floor(eta / 3600)).padStart(2, "0")}:${String(Math.floor((eta % 3600) / 60)).padStart(2, "0")}:${String(Math.floor(eta % 60)).padStart(2, "0")}`
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
		: isSettingUpProton
			? "Setting Up Environment..."
			: isVerifying
				? "Verifying Files..."
				: isCalculatingDownloads
					? "Calculating File Downloads..."
					: isFetchingManifest
						? "Fetching Manifest..."
						: state.downloadingGame !== null
							? `Downloading ${variantToGameName[state.downloadingGame]}...`
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
						variant="secondary"
						width={1}
						height={1}
					>
						{isPaused ? (
							<Play className={"-m-1 leading-0"} />
						) : (
							<Pause className={"-m-1 leading-0"} />
						)}
					</Button>
				)}
			</div>
			{isSettingUpProton && (
				<div class="flex min-w-full flex-col gap-y-1 text-left">
					<h2 class="ml-1 text-sm text-white">
						{protonPhaseLabel} {state.protonSetupComponent}
						{state.protonSetupPhase === "downloading" &&
						state.protonSetupDownloadTotal > 0
							? ` (${protonDownloadedMB}MB / ${protonTotalMB}MB - ${protonSetupPct.toFixed(1)}%)`
							: state.protonSetupPhase !== "downloading"
								? "..."
								: ""}
					</h2>
					<Progressbar progress={protonSetupPct} game={game} />
				</div>
			)}
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
						{formatNumber(state.totalFiles)} chunks (
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
};

export default DownloadProgress;
