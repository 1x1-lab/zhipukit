import * as THREE from 'three'

/** 徽章的圆角六边形外轮廓，scale = 1 时为成品尺寸 */
export function badgeShape(scale: number): THREE.Shape {
  const points = [
    [0, 27], [-23, 13.5], [-23, -13.5],
    [0, -27], [23, -13.5], [23, 13.5],
  ].map(([x, y]) => new THREE.Vector2(x * scale, y * scale))
  const radius = 2.4 * scale
  const shape = new THREE.Shape()

  points.forEach((point, index) => {
    const previous = points[(index + points.length - 1) % points.length]
    const next = points[(index + 1) % points.length]
    const incoming = previous.clone().sub(point).normalize().multiplyScalar(radius).add(point)
    const outgoing = next.clone().sub(point).normalize().multiplyScalar(radius).add(point)
    if (index === 0) shape.moveTo(incoming.x, incoming.y)
    else shape.lineTo(incoming.x, incoming.y)
    shape.quadraticCurveTo(point.x, point.y, outgoing.x, outgoing.y)
  })
  shape.closePath()
  return shape
}

export interface MedalProfile {
  /** 边框内侧轮廓相对外轮廓的比例，越接近 1 边框越窄 */
  frameInner: number
  /** 凹面的高度 */
  floorZ: number
  /** 边框顶面的高度 */
  topZ: number
  /** 背面的高度 */
  backZ: number
  /** 外沿倒角的高度 */
  bevel: number
}

/** 徽章各面所属的材质分组 */
export const MEDAL_MATERIAL = {
  /** 抛光边框：顶面与外沿倒角 */
  frame: 0,
  /** 侧壁、背面与凹槽内壁 */
  wall: 1,
  /** 喷砂凹面 */
  floor: 2,
} as const

type Part = { positions: number[]; indices: number[] }

/**
 * 一次建模出一整块徽章：外沿是凸起的窄边框，中间凹下去，底部相连。
 *
 * 沿高度把轮廓串成一个连续网格，自下而上依次是：
 *   背面 → 外侧壁 → 外沿倒角 → 边框顶面（外轮廓到内轮廓的环）→ 凹槽内壁 → 凹面
 * 相邻的面共用同一圈轮廓位置，所以整个徽章是一个整体，不是拼起来的零件。
 *
 * 每个面使用独立顶点，棱角处的法线不会被平均掉，边缘才是清晰的硬边。
 */
export function buildMedalGeometry(profile: MedalProfile): THREE.BufferGeometry {
  const { frameInner, floorZ, topZ, backZ, bevel } = profile
  // 一圈轮廓上的采样点；内外轮廓用同样的分段数，保证能一一对应
  const outline = (scale: number) => badgeShape(scale).getPoints(64)
  const outer = outline(1)
  const inner = outline(frameInner)
  const count = outer.length
  if (inner.length !== count) throw new Error('内外轮廓采样点数量不一致')

  const framePart: Part = { positions: [], indices: [] }
  const wallPart: Part = { positions: [], indices: [] }
  const floorPart: Part = { positions: [], indices: [] }

  // 把一圈轮廓放进某个分组，返回这一圈顶点在该分组内的起始下标
  function addRing(part: Part, points: THREE.Vector2[], z: number): number {
    const start = part.positions.length / 3
    for (const point of points) part.positions.push(point.x, point.y, z)
    return start
  }

  // badgeShape 的顶点是顺时针排列的（从顶部往左走）。下面的绕向已按实测校准：
  // 每个三角形的正面法线 = (b - a) × (c - a)，朝向由 measureFacing() 验证，不要凭感觉改。

  // 用上下两圈顶点围成一圈侧向的面。outward 为 true 时正面朝外（外侧壁、倒角），false 时正面朝内（凹槽内壁）。
  // 两个分支的顶点顺序必须互为相反绕向，否则参数不起作用
  function addWall(part: Part, lower: number, upper: number, outward: boolean) {
    for (let i = 0; i < count; i++) {
      const next = (i + 1) % count
      if (outward) part.indices.push(lower + i, lower + next, upper + next, lower + i, upper + next, upper + i)
      else part.indices.push(lower + i, upper + next, lower + next, lower + i, upper + i, upper + next)
    }
  }

  // 用内外两圈轮廓围成一个朝上的水平环面（边框顶面）。a 为外圈，b 为内圈
  function addRingFace(part: Part, a: number, b: number) {
    for (let i = 0; i < count; i++) {
      const next = (i + 1) % count
      part.indices.push(a + i, b + next, b + i, a + i, a + next, b + next)
    }
  }

  // 把一圈轮廓封成一个平面：facingUp 为 true 朝上（凹面），false 朝下（背面）
  function addCap(part: Part, start: number, facingUp: boolean) {
    for (let i = 1; i < count - 1; i++) {
      if (facingUp) part.indices.push(start, start + i, start + i + 1)
      else part.indices.push(start, start + i + 1, start + i)
    }
  }

  // 外沿倒角：边框顶面的外缘比最外侧略收一点，形成一圈抛光斜面
  const outerTop = outline(1 - bevel / 60)

  // ── 背面 ──
  const back = addRing(wallPart, outer, backZ)
  addCap(wallPart, back, false)

  // ── 外侧壁：从背面一直到倒角起点 ──
  const wallTop = addRing(wallPart, outer, topZ - bevel)
  addWall(wallPart, back, wallTop, true)

  // ── 外沿倒角：从侧壁顶收到边框顶面 ──
  const frameOuter = addRing(framePart, outerTop, topZ)
  const frameWallTop = addRing(framePart, outer, topZ - bevel)
  addWall(framePart, frameWallTop, frameOuter, true)

  // ── 边框顶面：外轮廓到内轮廓的环 ──
  const frameInnerTop = addRing(framePart, inner, topZ)
  addRingFace(framePart, frameOuter, frameInnerTop)

  // ── 凹槽内壁：从边框顶面下到凹面。它的正面朝向凹槽中心（朝内），与外侧壁相反 ──
  const wallInnerTop = addRing(wallPart, inner, topZ)
  const wallInnerFloor = addRing(wallPart, inner, floorZ)
  addWall(wallPart, wallInnerFloor, wallInnerTop, false)

  // ── 凹面 ──
  const floor = addRing(floorPart, inner, floorZ)
  addCap(floorPart, floor, true)

  // 合并三个分组，并记录每段索引的起始位置和长度
  const positions: number[] = []
  const indices: number[] = []
  const parts: { part: Part; material: number }[] = [
    { part: framePart, material: MEDAL_MATERIAL.frame },
    { part: wallPart, material: MEDAL_MATERIAL.wall },
    { part: floorPart, material: MEDAL_MATERIAL.floor },
  ]
  const geometry = new THREE.BufferGeometry()
  for (const { part, material } of parts) {
    const vertexOffset = positions.length / 3
    const indexStart = indices.length
    for (const value of part.positions) positions.push(value)
    for (const index of part.indices) indices.push(index + vertexOffset)
    geometry.addGroup(indexStart, indices.length - indexStart, material)
  }
  geometry.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3))
  geometry.setIndex(indices)
  geometry.computeVertexNormals()
  return geometry
}

/**
 * 检查几何各面的朝向是否正确：返回每个材质分组里三角形正面朝向的平均值。
 * 用于在修改绕向后验证，避免靠肉眼判断面是否被剔除。
 */
export function measureFacing(geometry: THREE.BufferGeometry): Record<'frame' | 'wall' | 'floor', { nx: number; ny: number; nz: number; triangles: number }> {
  const position = geometry.getAttribute('position')
  const index = geometry.getIndex()
  if (!index) throw new Error('几何缺少索引')
  const names = ['frame', 'wall', 'floor'] as const
  const result = {} as Record<'frame' | 'wall' | 'floor', { nx: number; ny: number; nz: number; triangles: number }>

  for (const group of geometry.groups) {
    const name = names[group.materialIndex ?? -1]
    if (!name) throw new Error(`几何分组使用了未知的材质下标: ${group.materialIndex}`)
    const sum = { nx: 0, ny: 0, nz: 0 }
    let triangles = 0
    const a = new THREE.Vector3(), b = new THREE.Vector3(), c = new THREE.Vector3()
    for (let i = group.start; i < group.start + group.count; i += 3) {
      a.fromBufferAttribute(position, index.getX(i))
      b.fromBufferAttribute(position, index.getX(i + 1))
      c.fromBufferAttribute(position, index.getX(i + 2))
      // 三角形正面法线 = (b - a) × (c - a)
      const n = b.sub(a).cross(c.sub(a))
      sum.nx += n.x
      sum.ny += n.y
      sum.nz += n.z
      triangles++
    }
    result[name] = { nx: sum.nx / triangles, ny: sum.ny / triangles, nz: sum.nz / triangles, triangles }
  }
  return result
}
