import { useEffect, useState } from "preact/hooks";
import { useDownload } from "../../hooks/useDownload";
import { useGame } from "../../hooks/useGame";
import {
	checkGameUpdate,
	downloadGame,
	downloadUpdate,
	isGameInstalled,
	resumeDownloadInterrupted,
	runGame,
} from "../../lib/GameDownloader";
import {
	protonEnvAvailable,
	updateAllProtonComponents,
} from "../../lib/ProtonManager";
import {
	gameCodeToVariant,
	variantToGameCode,
} from "../../lib/VariantConverter";
import type { GameCodes } from "../../types";
import Button from "../Button";

export const InstallerButton = () => {
	const { game } = useGame();
	const { state, setDownloadingGame, setResumable, setProtonSetupProgress } =
		useDownload();
	const [protonAvailable, setProtonAvailable] = useState<boolean>(false);
	const [gameInstalled, setGameInstalled] = useState<boolean>(false);
	const [updateAvailable, setUpdateAvailable] = useState<boolean>(false);
	const [preinstallDownloaded, setPreinstallDownloaded] =
		useState<boolean>(false);

	const downloadActive =
		state.isDownloading ||
		state.isAssembling ||
		state.isVerifying ||
		state.isFetchingManifest ||
		state.isPaused ||
		state.isApplyingPreinstall;
	const isDownloadForActiveGame = state.downloadingGame === game;
	const canResume =
		state.isResumable &&
		state.resumeInfo !== null &&
		variantToGameCode[game] === state.resumeInfo.gameId;

	useEffect(() => {
		let cancelled = false;
		protonEnvAvailable().then((res) => {
			if (!cancelled) setProtonAvailable(res);
		});
		isGameInstalled(game).then((res) => {
			if (!cancelled) setGameInstalled(res);
		});
		return () => {
			cancelled = true;
		};
	}, [game]);

	useEffect(() => {
		if (!gameInstalled) return;
		let cancelled = false;
		checkGameUpdate(game).then((res) => {
			if (!cancelled && res) {
				setUpdateAvailable(res.updateAvailable);
				setPreinstallDownloaded(res.preinstallDownloaded);
			}
		});
		return () => {
			cancelled = true;
		};
	}, [game, gameInstalled, state.isFinished]);

	useEffect(() => {
		if (state.isFinished && isDownloadForActiveGame) {
			setGameInstalled(true);
		}
	}, [state.isFinished, isDownloadForActiveGame]);

	const showUpdate = updateAvailable && gameInstalled && !preinstallDownloaded;

	const resumeVariant = state.resumeInfo
		? gameCodeToVariant[state.resumeInfo.gameId as GameCodes]
		: null;

	return (
		<div class="flex w-auto flex-row gap-x-3.5">
			<Button
				variant="primary"
				width={13.75}
				height={4.06}
				disabled={(downloadActive && !gameInstalled) || state.isSettingUpProton}
				onClick={async () => {
					if (!protonAvailable) {
						await updateAllProtonComponents((event) => {
							setProtonSetupProgress(event);
						});
						setProtonAvailable(true);
					} else if (canResume && resumeVariant !== null) {
						setResumable(null);
						setDownloadingGame(resumeVariant);
						await resumeDownloadInterrupted();
					} else if (!gameInstalled) {
						setDownloadingGame(game);
						await downloadGame(game);
					} else if (showUpdate) {
						setDownloadingGame(game);
						await downloadUpdate(game, false);
					} else {
						await runGame(game);
					}
				}}
			>
				{(() => {
					if (!protonAvailable) {
						return state.isSettingUpProton ? "Setting Up..." : "Create Env";
					} else if (canResume && !gameInstalled) {
						return downloadActive && isDownloadForActiveGame
							? "Downloading..."
							: "Resume Download";
					} else if (!gameInstalled) {
						return downloadActive && isDownloadForActiveGame
							? "Downloading..."
							: "Download";
					} else if (showUpdate) {
						return downloadActive && isDownloadForActiveGame
							? "Updating..."
							: "Update";
					} else {
						return "Play";
					}
				})()}
			</Button>
		</div>
	);
};

export default InstallerButton;
