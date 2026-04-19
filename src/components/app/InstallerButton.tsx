import { useEffect, useState } from "preact/hooks";
import Button from "../Button";
import { updateWineComponents, wineEnvAvailable } from "../../lib/WineManager";
import {
	downloadGame,
	isGameInstalled,
	pauseDownload,
	resumeDownload,
	runGame,
} from "../../lib/GameDownloader";
import { useGame } from "../../hooks/useGame";
import { SophonProgress, Variants } from "../../types";
import { listen } from "@tauri-apps/api/event";
import { Pause, Play } from "lucide-preact";

const downloadMatchesActiveGame = (
	downloadingGame: Variants | null,
	activeGame: Variants,
) => {
	console.log(downloadingGame);
	console.log(activeGame);
	return (
		downloadingGame !== null && (downloadingGame as Variants) === activeGame
	);
};

export default function InstallerButton() {
	const { game } = useGame();

	let [wineAvailable, setWineAvailable] = useState<boolean>(false);
	let [gameInstalled, setGameInstalled] = useState<boolean>(false);

	let [currentGameDownload, setCurrentGameDownload] = useState<Variants | null>(
		null,
	);

	let [updatesAvailable, setUpdatesAvailable] = useState<boolean>(false);
	let [downloadInProgress, setDownloadInProgress] = useState<boolean>(false);
	let [downloadPaused, setDownloadPaused] = useState<boolean>(false);

	useEffect(() => {
		const unlisten = listen("sophon://progress", (event) => {
			const payload = event.payload as SophonProgress;
			if (
				["downloading", "assembling", "fetchingManifest", "paused"].includes(
					payload.type,
				)
			) {
				setDownloadInProgress(true);
			} else setDownloadInProgress(false);

			if (payload.type === "finished") {
				if (downloadMatchesActiveGame(currentGameDownload, game)) {
					setGameInstalled(true);
				}
				setCurrentGameDownload(null);
			}
		});
		return async () => {
			(await unlisten)();
		};
	}, []);

	useEffect(() => {
		wineEnvAvailable().then((res) => {
			setWineAvailable(res);
		});
		isGameInstalled(game).then((res) => {
			setGameInstalled(res);
		});
	}, [game]);

	return (
		<div class="w-auto flex flex-row gap-x-3.5">
			{downloadInProgress ? (
				<Button
					onClick={async () => {
						if (downloadPaused) {
							await resumeDownload();
							setDownloadPaused(false);
						} else {
							await pauseDownload();
							setDownloadPaused(true);
						}
					}}
					intent="secondary"
					iconButton>
					{!downloadPaused ? (
						<Pause className={"leading-0 -m-1"} />
					) : (
						<Play className={"leading-0 -m-1"} />
					)}
				</Button>
			) : null}
			<Button
				intent="primary"
				disabled={downloadInProgress && !gameInstalled}
				onClick={async () => {
					console.log("Clicked");
					if (!wineAvailable) {
						await updateWineComponents();
						setWineAvailable(true);
					} else if (!gameInstalled) {
						const activeGame = game;
						await downloadGame(activeGame);
					} else {
						await runGame(game);
					}
				}}>
				{(() => {
					if (!wineAvailable) {
						return "Create Env"; // TODO: Maybe replace this state with a onboarding screen in a later update
					} else if (!gameInstalled) {
						return downloadInProgress ? "Downloading..." : "Download";
					} else {
						return "Play";
					}
				})()}
			</Button>
		</div>
	);
}
