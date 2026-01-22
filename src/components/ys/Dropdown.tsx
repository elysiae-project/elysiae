import { Signal } from "@preact/signals";

export default function YsDropdown({
	labels,
	selectedIndex,
	onChangeAction,
}: {
	labels: string[];
	selectedIndex: number;
	onChangeAction: () => void;
}) {
	const selectedItem = new Signal<number>(selectedIndex);
	const onChange = () => {
		onChangeAction();
	};

	return (
		<div class="relative w-60">
			<div class="flex flex-col h-auto"></div>
		</div>
	);
}
