export type FocusDirection = 'left' | 'right' | 'up' | 'down';

export interface FocusTarget {
  id: string;
  zone: string;
  rect: {
    left: number;
    top: number;
    width: number;
    height: number;
  };
}

export interface FocusActionMap {
  activate: string[];
  back: string[];
  search: string[];
  menu: string[];
}

export const DEFAULT_FOCUS_ACTIONS: FocusActionMap = {
  activate: ['Enter', 'Space', 'GamepadA'],
  back: ['Escape', 'Backspace', 'GamepadB'],
  search: ['KeyX', 'GamepadX'],
  menu: ['KeyY', 'GamepadY']
};

export function nextFocusTarget(
  targets: FocusTarget[],
  currentId: string | null,
  direction: FocusDirection
): FocusTarget | null {
  if (targets.length === 0) return null;
  if (!currentId) return targets[0] ?? null;

  const current = targets.find((target) => target.id === currentId);
  if (!current) return targets[0] ?? null;

  const currentCenter = center(current);
  const candidates = targets
    .filter((target) => target.id !== current.id)
    .map((target) => ({
      target,
      center: center(target)
    }))
    .filter(({ center: candidateCenter }) => isCandidateInDirection(currentCenter, candidateCenter, direction))
    .map(({ target, center: candidateCenter }) => ({
      target,
      score: scoreCandidate(currentCenter, candidateCenter, direction)
    }))
    .sort((a, b) => a.score - b.score);

  return candidates[0]?.target ?? wrapWithinZone(targets, current, direction);
}

export function actionForInput(code: string, actions: FocusActionMap = DEFAULT_FOCUS_ACTIONS) {
  if (actions.activate.includes(code)) return 'activate';
  if (actions.back.includes(code)) return 'back';
  if (actions.search.includes(code)) return 'search';
  if (actions.menu.includes(code)) return 'menu';
  return null;
}

function center(target: FocusTarget) {
  return {
    x: target.rect.left + target.rect.width / 2,
    y: target.rect.top + target.rect.height / 2
  };
}

function isCandidateInDirection(
  current: { x: number; y: number },
  candidate: { x: number; y: number },
  direction: FocusDirection
) {
  if (direction === 'left') return candidate.x < current.x;
  if (direction === 'right') return candidate.x > current.x;
  if (direction === 'up') return candidate.y < current.y;
  return candidate.y > current.y;
}

function scoreCandidate(
  current: { x: number; y: number },
  candidate: { x: number; y: number },
  direction: FocusDirection
) {
  const primary = direction === 'left' || direction === 'right'
    ? Math.abs(candidate.x - current.x)
    : Math.abs(candidate.y - current.y);
  const secondary = direction === 'left' || direction === 'right'
    ? Math.abs(candidate.y - current.y)
    : Math.abs(candidate.x - current.x);

  return primary * 3 + secondary;
}

function wrapWithinZone(
  targets: FocusTarget[],
  current: FocusTarget,
  direction: FocusDirection
) {
  const zoneTargets = targets.filter((target) => target.zone === current.zone);
  if (zoneTargets.length === 0) return targets[0] ?? null;

  const sorted = [...zoneTargets].sort((a, b) => {
    if (direction === 'left' || direction === 'right') {
      return center(a).x - center(b).x;
    }
    return center(a).y - center(b).y;
  });

  return direction === 'left' || direction === 'up'
    ? sorted[sorted.length - 1] ?? null
    : sorted[0] ?? null;
}
