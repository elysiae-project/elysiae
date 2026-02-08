export default function SrModal({
	children,
	title,
	open,
}: {
	children: any;
	title: string;
	open: boolean;
}) {
	return (
		<>
			{children}
			{title}
		</>
	);
}
