// Parses the binary snapshot the WASM simulation rebuilds each frame. The read
// order here must exactly mirror `build_snapshot` in game/lib.rs.

export interface Entity {
  kind: number; // 1 monster, 2 player projectile, 3 monster projectile, 4 loot
  x: number;
  y: number;
  radius: number;
  hpFrac: number; // 0..1, 1 = full/none
  shape: number; // monster body / loot rarity
  dtype: number; // damage type / element
}

export interface EquipInfo {
  present: boolean;
  rarity: number;
  dtype: number;
  durability: number; // 0..100
  name: string;
}

export interface InvItem {
  rarity: number;
  dtype: number;
  base: number;
  damage: number;
  cooldown: number;
  equippedSlot: number; // 0..3, or 255
  durability: number; // 0..100
  unique: boolean;
  name: string;
}

export interface Target {
  name: string;
  level: number;
  weak: number;
  resist: number;
  hpFrac: number;
}

export interface Snapshot {
  camX: number;
  camY: number;
  tx0: number;
  ty0: number;
  cols: number;
  rows: number;
  tiles: Uint8Array;
  features: Uint8Array;
  px: number;
  py: number;
  hp: number;
  maxhp: number;
  dist: number;
  difficulty: number;
  ammo: number;
  maxDist: number;
  checkpointDist: number;
  celebrating: boolean;
  celebrateT: number;
  milestoneT: number;
  skillLevels: number[]; // length 8
  skillXp: number[]; // length 8
  slot: number;
  equipped: EquipInfo[]; // length 4
  entities: Entity[];
  target: Target | null;
  message: string;
  inventory: InvItem[];
  stats: { kills: number; deaths: number; chests: number; fountains: number; steps: number; playSecs: number; bossKills: number };
  arena: { active: boolean; wave: number; waves: number; rot: boolean; countdown: number; near: boolean };
  relic: { active: boolean; steps: number; stepsMax: number; shield: number; shieldMax: number; weapon: string };
  rest: { active: boolean; safe: boolean };
  shrine: boolean;
  canFish: boolean;
  shield: number;
  shieldMax: number;
  atVault: boolean;
}

const decoder = new TextDecoder();

class Reader {
  private p = 0;
  constructor(private dv: DataView, private bytes: Uint8Array) {}
  u8(): number {
    return this.dv.getUint8(this.p++);
  }
  u16(): number {
    const v = this.dv.getUint16(this.p, true);
    this.p += 2;
    return v;
  }
  i32(): number {
    const v = this.dv.getInt32(this.p, true);
    this.p += 4;
    return v;
  }
  f32(): number {
    const v = this.dv.getFloat32(this.p, true);
    this.p += 4;
    return v;
  }
  str(): string {
    const n = this.u8();
    const s = decoder.decode(this.bytes.subarray(this.p, this.p + n));
    this.p += n;
    return s;
  }
  slice(n: number): Uint8Array {
    const s = this.bytes.subarray(this.p, this.p + n);
    this.p += n;
    return s;
  }
}

export function parseSnapshot(buffer: ArrayBuffer, ptr: number, len: number): Snapshot {
  const bytes = new Uint8Array(buffer, ptr, len);
  const dv = new DataView(buffer, ptr, len);
  const r = new Reader(dv, bytes);

  const camX = r.f32();
  const camY = r.f32();
  const tx0 = r.i32();
  const ty0 = r.i32();
  const cols = r.u16();
  const rows = r.u16();
  const tiles = r.slice(cols * rows).slice(); // copy out of live wasm memory
  const features = r.slice(cols * rows).slice();

  const px = r.f32();
  const py = r.f32();
  const hp = r.f32();
  const maxhp = r.f32();
  const dist = r.f32();
  const difficulty = r.u16();
  const ammo = r.u16();
  const maxDist = r.f32();
  const checkpointDist = r.f32();
  const celebrating = r.u8() !== 0;
  const celebrateT = r.f32();
  const milestoneT = r.f32();

  const skillLevels: number[] = [];
  const skillXp: number[] = [];
  for (let i = 0; i < 8; i++) {
    skillLevels.push(r.u16());
    skillXp.push(r.f32());
  }
  const slot = r.u8();

  const equipped: EquipInfo[] = [];
  for (let i = 0; i < 4; i++) {
    if (r.u8() === 1) {
      equipped.push({ present: true, rarity: r.u8(), dtype: r.u8(), durability: r.u8(), name: r.str() });
    } else {
      equipped.push({ present: false, rarity: 0, dtype: 0, durability: 0, name: "" });
    }
  }

  const entities: Entity[] = [];
  const count = r.u16();
  for (let i = 0; i < count; i++) {
    entities.push({
      kind: r.u8(),
      x: r.f32(),
      y: r.f32(),
      radius: r.u8(),
      hpFrac: r.u8() / 255,
      shape: r.u8(),
      dtype: r.u8(),
    });
  }

  let target: Target | null = null;
  if (r.u8() === 1) {
    target = { name: r.str(), level: r.u16(), weak: r.u8(), resist: r.u8(), hpFrac: r.u8() / 255 };
  }

  const message = r.str();

  const inventory: InvItem[] = [];
  const invCount = r.u16();
  for (let i = 0; i < invCount; i++) {
    inventory.push({
      rarity: r.u8(),
      dtype: r.u8(),
      base: r.u8(),
      damage: r.f32(),
      cooldown: r.f32(),
      equippedSlot: r.u8(),
      durability: r.u8(),
      unique: r.u8() !== 0,
      name: r.str(),
    });
  }

  const stats = {
    kills: r.i32(),
    deaths: r.i32(),
    chests: r.i32(),
    fountains: r.i32(),
    steps: r.f32(),
    playSecs: r.f32(),
    bossKills: r.i32(),
  };

  const arena = { active: r.u8() === 1, wave: r.u8(), waves: r.u8(), rot: r.u8() === 1, countdown: r.u8(), near: r.u8() === 1 };

  const relic = {
    active: r.u8() === 1,
    steps: r.u16(),
    stepsMax: r.u16(),
    shield: r.f32(),
    shieldMax: r.f32(),
    weapon: r.str(),
  };

  const rest = { active: r.u8() === 1, safe: r.u8() === 1 };
  const shrine = r.u8() === 1;
  const canFish = r.u8() === 1;
  const shield = r.f32();
  const shieldMax = r.f32();
  const atVault = r.u8() === 1;

  return {
    camX, camY, tx0, ty0, cols, rows, tiles, features,
    px, py, hp, maxhp, dist, difficulty, ammo, maxDist, checkpointDist,
    celebrating, celebrateT, milestoneT,
    skillLevels, skillXp, slot, equipped, entities, target, message, inventory, stats, arena, relic, rest, shrine, canFish,
    shield, shieldMax, atVault,
  };
}
