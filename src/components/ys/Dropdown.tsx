import { ArrowDown, Check } from "lucide-preact";
import { MutableRef, useEffect, useRef, useState } from "preact/hooks";

export default function YsDropdown({
	labels,
	initialIndex = 0,
	onChangeAction,
}: {
	labels: string[];
	initialIndex: number;
	onChangeAction: (index: number) => void;
}) {
	let [open, setOpen] = useState<boolean>(false);
	let [label, setLabel] = useState<string>(labels[initialIndex]);
	let [currentIndex, setCurrentIndex] = useState<number>(initialIndex);

	const dropdownDiv: MutableRef<HTMLDivElement | null> = useRef(null);

	const onChange = (index: number) => {
		if (!open) return; // Permanent temp fix lol
		setLabel(labels[index]);
		setOpen(false);
		setCurrentIndex(index);
		onChangeAction(index);
	};

	useEffect(() => {
		const onClick = (e: MouseEvent) => {
			setOpen(false);
		};
		if (open) {
			document.addEventListener("click", onClick);
		}
		return () => {
			document.removeEventListener("click", onClick);
		};
	}, [open]);

	return (
		<div class="relative w-60" ref={dropdownDiv} style={{ zIndex: 500 }}>
			<div class="overflow-y-auto-y flex h-full flex-col">
				<div
					class="flex flex-row h-10 px-3 items-center justify-between rounded-3xl border-2 border-transparent bg-[#dcd5c9] hover:border-white active:border-[#444140] active:bg-[#edd4b2]"
					onClick={(e) => {
						e.stopPropagation();
						setOpen(!open);
					}}
				>
					<h1 class="text-[#3d4557]">{label}</h1>
					<ArrowDown />
				</div>
				<div
					class="min-h-auto mt-10.5 absolute flex w-full flex-col rounded-[1.25rem] bg-[#495366] drop-shadow-md px-1  py-1 transition-opacity duration-150"
					style={open ? "opacity: 100" : "opacity: 0;"}
				>
					{labels.map((label, index) => {
						return (
							<div
								onClick={() => onChange(index)}
								class="bg-transparent hover:bg-[#606979] active:bg-[#ece5d8] rounded-4xl text-[#ece5d8] active:text-[#495366] w-full h-8 py-1 px-2 flex flex-row justify-between *:items-center text-center"
							>
								<p class="pt-0.5">{label}</p>
								{index === currentIndex ? (
									<Check style="color: #ece5d" />
								) : null}
							</div>
						);
					})}
				</div>
			</div>
		</div>
	);
}
