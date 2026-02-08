import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import BhProgressbar from "./bh/Progressbar";
import NapProgressbar from "./nap/Progressbar";
import SrProgressbar from "./sr/Progressbar";
import YsProgressbar from "./ys/Progressbar";

export default function Progressbar({ progress }: { progress: number }) {
	const game = useGame();
	const Progressbar = {
		[Variants.BH]: BhProgressbar,
		[Variants.YS]: YsProgressbar,
		[Variants.SR]: SrProgressbar,
		[Variants.NAP]: NapProgressbar,
	}[game];
	return (
		<Progressbar
			progress={progress}
		></Progressbar>
	);
}
