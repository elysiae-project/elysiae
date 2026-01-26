import { cva } from "class-variance-authority";

const button = cva(
	"rounded-full min-w-10 px-2 py-1.5 font-sr outline-2 hover:outline-[#fcfcfc] active:outline-[#fcfcfc] transition duration-250 drop-shadow-sm",
	{
		variants: {
			intent: {
				primary:
					"bg-[#f3c559] outline-[#f2b316] hover:bg-[#f6d477] active:bg-[#e4b252]",
				secondary: "bg-[#dddddd] outline-[#00000000] hover:bg-white active:bg-[#c8c8c8]",
			},
		},
	},
);

export default function SrButton({
	onClick,
	children,
	intent,
}: {
	onClick: () => void;
	children: any;
	intent: "primary" | "secondary" | null | undefined;
}) {
	return (
		<div class={button({ intent: intent })} onClick={onClick}>
			<div class="bg-transparent rounded-full p-2.5 flex flex-row gap-3 justify-center items-center">{children}</div>
		</div>
	);
}
