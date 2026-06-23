import { useEffect, useState } from "preact/hooks";
import { useDownload } from "../../hooks/useDownload";
import { useGame } from "../../hooks/useGame";
import {
	downloadGame,
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
				disabled={(downloadActive && !gameInstalled) || state.isSettingUpProton}
				onClick={async () => {
					if (!protonAvailable) {
						try {
							await updateAllProtonComponents((event) => {
								setProtonSetupProgress(event);
							});
							setProtonAvailable(await protonEnvAvailable());
						} catch {
							setProtonAvailable(false);
						}
					} else if (canResume && resumeVariant !== null) {
						setResumable(null);
						setDownloadingGame(resumeVariant);
						await resumeDownloadInterrupted();
					} else if (!gameInstalled) {
						setDownloadingGame(game);
						try {
							await downloadGame(game);
						} catch {
							// Error state is set via sophon://error listener
						}
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
					} else {
						return "Play";
					}
				})()}
			</Button>
		</div>
	);
};

export default InstallerButton;
