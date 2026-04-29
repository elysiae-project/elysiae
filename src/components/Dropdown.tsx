import { cva } from "class-variance-authority";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { MutableRef, useEffect, useRef, useState } from "preact/hooks";
import { ArrowDown, Check } from "lucide-preact";

const dropdownStyles = cva(
	"flex flex-row h-10 px-3 items-center justify-between transition duration-150",
	{
		variants: {
			game: {
				[Variants.BH3]:
					"bg-bh3-dropdown text-bh3-dropdown-text active:bg-bh3-dropdown-active",
				[Variants.HK4E]:
					"rounded-3xl border-2 border-transparent bg-hk4e-dropdown hover:border-white active:border-hk4e-dropdown-border-active active:bg-hk4e-dropdown-active text-hk4e-dropdown-text",
				[Variants.HKRPG]:
					"rounded-full bg-hkrpg-dropdown-bg hover:bg-white border-2 border-transparent hover:border-hkrpg-dropdown-hover-border active:border-white active:bg-hkrpg-dropdown-active-bg",
				[Variants.NAP]:
					"nap-dots rounded-full border-3 border-[#353535] active:animate-nap-pulsate",
			},
		},
	},
);

const dropdownList = cva(
	"min-h-auto mt-10.5 absolute flex w-full flex-col transition-opacity duration-150",
	{
		variants: {
			game: {
				[Variants.BH3]: "bg-white text-bh-dropdown-text",
				[Variants.HK4E]:
					"rounded-[1.25rem] bg-[#495366] drop-shadow-md px-1 py-1 transition-opacity duration-150",
				[Variants.HKRPG]: "bg-sr-list-bg mt-12 rounded-xs",
				[Variants.NAP]: "bg-[#353535] rounded-2xl",
			},
		},
	},
);

const dropdownItem = cva(
	"w-full h-8 py-1 px-2 flex flex-row bg-transparent justify-between items-center text-center transition-all duration-175",
	{
		variants: {
			game: {
				[Variants.BH3]: "active:bg-bh-dropdown-item-active",
				[Variants.HK4E]:
					"hover:bg-ys-item-hover active:bg-[#ece5d8] rounded-4xl text-[#ece5d8] active:text-[#495366]",
				[Variants.HKRPG]:
					"py-5 active:bg-sr-item-active-bg hover:bg-sr-item-hover-bg",
				[Variants.NAP]:
					"text-white active:text-black active:animate-nap-pulsate rounded-full text-center",
			},
		},
	},
);

export default function Dropdown({
	labels,
	initialIndex = 0,
	onChangeAction,
	width = 120,
	height = 20,
}: {
	labels: string[];
	initialIndex: number;
	onChangeAction: (label: string) => void;
	width?: number;
	height?: number;
}) {
	const { game } = useGame();

	let [open, setOpen] = useState<boolean>(false);
	let [label, setLabel] = useState<string>(labels[initialIndex]);
	let [currentIndex, setCurrentIndex] = useState<number>(initialIndex);

	const dropdownDiv: MutableRef<HTMLDivElement | null> = useRef(null);

	const onChange = (index: number) => {
		if (!open) return;
		setLabel(labels[index]);
		setOpen(false);
		setCurrentIndex(index);
		onChangeAction(label);
	};

	useEffect(() => {
		const onClick = () => {
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
		<div
			class="relative"
			ref={dropdownDiv}
			style={{ zIndex: 80, minWidth: `${width}px`, minHeight: `${height}px` }}>
			<div class="flex h-full flex-col overflow-y-auto">
				<div
					class={dropdownStyles({ game: game })}
					onClick={(e) => {
						e.stopPropagation();
						setOpen(!open);
					}}>
					<h1>{label}</h1>
					<ArrowDown />
				</div>
				<div
					class={dropdownList({ game: game })}
					style={open ? "opacity: 100" : "opacity: 0;"}>
					{labels.map((listLabel, index) => {
						return (
							<div
								onClick={() => onChange(index)}
								class={dropdownItem({ game: game })}>
								<p class="pt-0.5">{listLabel}</p>
								{index === currentIndex ? <Check /> : null}
							</div>
						);
					})}
				</div>
			</div>
		</div>
	);
}
