/**
 * A persona's avatar: the app's portrait shape, 3:4. Empty for now, a sunken frame the avatar generator will fill;
 * each place sizes it. `title` names whose it is on hover.
 */
export function Avatar({ title }: { title?: string }) {
  return <span className="avatar" title={title} aria-hidden="true" />;
}
