import { Play, Save } from "lucide-preact";
import { useEffect, useState } from "preact/hooks";
import { useDownload } from "../../hooks/useDownload";
import { useGame } from "../../hooks/useGame";
import {
	applyUpdate,
	downloadUpdate,
	getPreinstallState,
} from "../../lib/GameDownloader";
import type { PreinstallState } from "../../lib/GameDownloader";
import Button from "../Button";

export const PreinstallButton = () => {
	const [preinstallState, setPreinstallState] =
		useState<PreinstallState>("hidden");
	const { game } = useGame();
	const { state: dlState, setDownloadingGame } = useDownload();
	const downloadActive =
		dlState.isDownloading ||
		dlState.isAssembling ||
		dlState.isVerifying ||
		dlState.isFetchingManifest ||
		dlState.isPaused;

	useEffect(() => {
		let cancelled = false;
		getPreinstallState(game).then((result) => {
			if (!cancelled) setPreinstallState(result);
		});
		return () => {
			cancelled = true;
		};
	}, [game, dlState.isFinished]);

	if (preinstallState === "hidden") return null;

	if (preinstallState === "apply") {
		return (
			<Button
				variant="secondary"
				width={4}
				height={4}
				disabled={downloadActive}
				onClick={async () => {
					setDownloadingGame(game);
					await applyUpdate(game);
				}}
			>
				<Play className="-m-1 leading-0" />
			</Button>
		);
	}

	return (
		<Button
			variant="secondary"
			width={4}
			height={4}
			disabled={downloadActive}
			onClick={async () => {
				setDownloadingGame(game);
				await downloadUpdate(game, true);
			}}
		>
			<Save className="-m-1 leading-0" />
		</Button>
	);
};

export default PreinstallButton;
