import { Signal } from "@preact/signals";
import { MutableRef, useRef } from "preact/hooks";


export default function YsDropdown({
	labels,
	selectedIndex,
	onChangeAction,
}: {
	labels: string[];
	selectedIndex: number;
	onChangeAction: () => void;
}) {
	const dropdownElement: MutableRef<null | HTMLDivElement> = useRef(null)
	const selectedItem = new Signal<number>(selectedIndex);
	const currentLabel = new Signal<string>(labels[selectedItem.value]);

	const onChange = (index: number) => {

		onChangeAction();
	};

	return (
		<div class="relative w-60">
			<div class="flex flex-col h-auto overflow-y-auto bg-[#dcd5c9]" ref={dropdownElement}>
				<h1>{currentLabel}</h1>

				{labels.map((label, index) => {
					return(
						<>
						</>
					)
				})}
			</div>
		</div>
	);
}
