import { useEffect, useState } from "preact/hooks";
import { useDownload } from "../../hooks/useDownload";
import { useGame } from "../../hooks/useGame";
import { gameCodeToVariant, variantToGameCode } from "../../lib/AppFunctions";
import {
	downloadGame,
	isGameInstalled,
	resumeDownloadInterrupted,
	runGame,
} from "../../lib/GameDownloader";
import {
	updateAllWineComponents,
	wineEnvAvailable,
} from "../../lib/WineManager";
import type { GameCodes } from "../../types";
import Button from "../Button";

export const InstallerButton = () => {
	const { game } = useGame();
	const { state, setDownloadingGame, setResumable, setWineSetupProgress } =
		useDownload();
	const [wineAvailable, setWineAvailable] = useState<boolean>(false);
	const [gameInstalled, setGameInstalled] = useState<boolean>(false);

	const downloadActive =
		state.isDownloading ||
		state.isAssembling ||
		state.isVerifying ||
		state.isFetchingManifest ||
		state.isPaused;
	const isDownloadForActiveGame = state.downloadingGame === game;
	const canResume =
		state.isResumable &&
		state.resumeInfo !== null &&
		variantToGameCode[game] === state.resumeInfo.gameId;

	useEffect(() => {
		let cancelled = false;
		wineEnvAvailable().then((res) => {
			if (!cancelled) setWineAvailable(res);
		});
		isGameInstalled(game).then((res) => {
			if (!cancelled) setGameInstalled(res);
		});
		return () => {
			cancelled = true;
		};
	}, [game]);

	useEffect(() => {
		if (state.isFinished && isDownloadForActiveGame) {
			setGameInstalled(true);
		}
	}, [state.isFinished, isDownloadForActiveGame]);

	const resumeVariant = state.resumeInfo
		? gameCodeToVariant[state.resumeInfo.gameId as GameCodes]
		: null;

	return (
		<div class="flex w-auto flex-row gap-x-3.5">
			<Button
				variant="primary"
				width={13.75}
				height={4.06}
				disabled={(downloadActive && !gameInstalled) || state.isSettingUpWine}
				onClick={async () => {
					if (!wineAvailable) {
						await updateAllWineComponents((event) => {
							setWineSetupProgress(event);
						});
						setWineAvailable(true);
					} else if (canResume && resumeVariant !== null) {
						setResumable(null);
						setDownloadingGame(resumeVariant);
						await resumeDownloadInterrupted();
					} else if (!gameInstalled) {
						setDownloadingGame(game);
						await downloadGame(game);
					} else {
						await runGame(game);
					}
				}}
			>
				{(() => {
					if (!wineAvailable) {
						return state.isSettingUpWine ? "Setting Up..." : "Create Env";
					} else if (canResume && !gameInstalled) {
						return downloadActive && isDownloadForActiveGame
							? "Downloading..."
							: "Resume Download";
					} else if (!gameInstalled) {
						return downloadActive && isDownloadForActiveGame
							? "Downloading..."
							: "Download";
					} else {
						return "Play";
					}
				})()}
			</Button>
		</div>
	);
};

export default InstallerButton;
