import { useEffect } from "preact/hooks";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";

import BhToggleSwitch from "./bh/ToggleSwitch";
import NapToggleSwitch from "./nap/ToggleSwitch";
import SrToggleSwitch from "./sr/ToggleSwitch";
import YsToggleSwitch from "./ys/ToggleSwitch";

export default function ToggleSwitch({
	onClick,
	startActive = false,
}: {
	onClick: (enabled: boolean) => void;
	startActive: boolean;
}) {
	useEffect(() => {
		console.log("LOADED");
	}, []);

	const game = useGame();
	const ToggleSwitch = {
		[Variants.BH]: BhToggleSwitch,
		[Variants.YS]: YsToggleSwitch,
		[Variants.SR]: SrToggleSwitch,
		[Variants.NAP]: NapToggleSwitch,
	}[game];

	return (
		<ToggleSwitch
			onClick={(e) => onClick(e)}
			startActive={startActive}
		></ToggleSwitch>
	);
}
