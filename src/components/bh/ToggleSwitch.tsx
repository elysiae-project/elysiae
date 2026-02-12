import { useState } from "preact/hooks";

export default function BhToggleSwitch({
	onClick,
	startActive,
}: {
	onClick: (enabled: boolean) => void;
	startActive: boolean;
}) {
	let [enabled, setEnabled] = useState<boolean>(startActive);
	return <>Placeholder</>;
}
