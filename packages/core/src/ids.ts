let generation = 0;
let seq = 0;

export function currentGeneration(): number {
  return generation;
}

export function newGeneration(): void {
  generation += 1;
  seq = 0;
}

/** Generation-tagged monotonic node id: (generation << 24) | (seq & 0xFFFFFF). */
export function nextNodeId(): number {
  seq += 1;
  return (generation << 24) | (seq & 0xffffff);
}
