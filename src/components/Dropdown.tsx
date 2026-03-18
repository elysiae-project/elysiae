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
					"bg-bh-dropdown text-bh-dropdown-text active:bg-bh-dropdown-active",
				[Variants.HK4E]:
					"rounded-3xl border-2 border-transparent bg-ys-dropdown hover:border-white active:border-ys-dropdown-border-active active:bg-ys-dropdown-active text-ys-dropdown-text",
				[Variants.HKRPG]:
					"rounded-full bg-sr-dropdown-bg hover:bg-white border-2 border-transparent hover:border[#eeeeee] active:border-white active:bg-sr-dropdown-active-bg",
				[Variants.NAP]:
					"nap-dots rounded-full border-3 border-[#353535] active:animate-nap-pulsate",
			},
		},
	},
);

const dropdownListStyles = cva(
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

const dropdownItemStyles = cva(
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
}: {
	labels: string[];
	initialIndex: number;
	onChangeAction: (index: number) => void;
}) {
	const { game, setGame } = useGame();

	let [open, setOpen] = useState<boolean>(false);
	let [label, setLabel] = useState<string>(labels[initialIndex]);
	let [currentIndex, setCurrentIndex] = useState<number>(initialIndex);

	const dropdownDiv: MutableRef<HTMLDivElement | null> = useRef(null);

	const onChange = (index: number) => {
		if (!open) return;
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
			<div class="flex h-full flex-col overflow-y-auto">
				<div
					class={dropdownStyles({ game: game })}
					onClick={(e) => {
						e.stopPropagation();
						setOpen(!open);
					}}
				>
					<h1>{label}</h1>
					<ArrowDown />
				</div>
				<div
					class={dropdownListStyles({ game: game })}
					style={open ? "opacity: 100" : "opacity: 0;"}
				>
					{labels.map((listLabel, index) => {
						return (
							<div
								onClick={() => onChange(index)}
								class={dropdownItemStyles({ game: game })}
							>
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
