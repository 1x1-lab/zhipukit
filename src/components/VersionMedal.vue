<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import * as THREE from 'three'
import { FontLoader } from 'three/addons/loaders/FontLoader.js'
import { TextGeometry } from 'three/addons/geometries/TextGeometry.js'
import fontData from '../assets/helvetiker_bold.typeface.json'
import { MEDAL_MATERIAL, badgeShape, buildMedalGeometry, type MedalProfile } from './medalGeometry'

const props = defineProps<{ version: string }>()

const expanded = ref(false)
/** 关闭动画进行中：此时徽章仍留在浮层里播放回程动画，结束后才真正收起 */
const closing = ref(false)
const failed = ref(false)
const canvas = ref<HTMLCanvasElement | null>(null)
const root = ref<HTMLButtonElement | null>(null)

const isV3 = computed(() => props.version.toLowerCase() === 'v3')
const text = computed(() => isV3.value ? '3' : '2')
const description = computed(() => isV3.value ? 'V3 积分制套餐徽章' : 'V2 按量套餐徽章')

let disposeBadge: (() => void) | null = null

function closeOnEscape(event: KeyboardEvent) {
  if (event.key === 'Escape') close()
}

/** 徽章的尺寸：整块金属的厚度、边框高度、凹面深度都在这一个轮廓里 */
const MEDAL: MedalProfile = {
  // 边框很窄：内侧轮廓只比外轮廓收一点
  frameInner: 0.91,
  // 凹面低于边框顶面，V3 从这里凸起
  floorZ: 2.0,
  topZ: 4.3,
  backZ: -1.7,
  bevel: 0.35,
}

/** 小勋章在页面上的显示尺寸（CSS 像素） */
const SMALL_SIZE = 28
/** 点击放大后的显示尺寸（CSS 像素） */
const EXPANDED_SIZE = 420
/** 放大状态下徽章的缩放倍数，由上面两个尺寸算出。
 *  样式里 .version-medal.is-expanded 的 scale 用的是同一个变量，进出动画换算起点位置时也要用它，
 *  所以必须定义在所有使用它的代码之前 */
const EXPANDED_SCALE = EXPANDED_SIZE / SMALL_SIZE

function sandblastedTexture(): THREE.DataTexture {
  const size = 256
  const data = new Uint8Array(size * size * 4)
  let seed = 41
  const random = () => {
    seed = (seed * 16807) % 2147483647
    return seed / 2147483647
  }

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const grain = (random() - 0.5) * 88
      const soft = Math.sin(x * 0.32 + y * 0.15) * 12 + Math.sin(y * 0.45) * 10
      const value = Math.max(0, Math.min(255, 176 + grain + soft))
      const offset = (y * size + x) * 4
      data[offset] = value
      data[offset + 1] = value
      data[offset + 2] = value
      data[offset + 3] = 255
    }
  }

  const texture = new THREE.DataTexture(data, size, size, THREE.RGBAFormat)
  texture.needsUpdate = true
  texture.wrapS = THREE.RepeatWrapping
  texture.wrapT = THREE.RepeatWrapping
  texture.repeat.set(0.035, 0.035)
  return texture
}

function initBadge(canvas: HTMLCanvasElement, target: HTMLElement) {
  const renderer = new THREE.WebGLRenderer({
    canvas,
    alpha: true,
    antialias: true,
    powerPreference: 'high-performance',
  })
  // 渲染分辨率必须跟随屏幕像素比。写死为 1 时，在 Retina 等高分屏上（1 个 CSS 像素 = 2~3 个物理像素）
  // 画布像素不够，放大会被拉伸得模糊、显得又小又糊。
  // 画布的逻辑边长取放大后的显示尺寸，再乘以像素比；上限 3 是为了不让 4K 高缩放屏占用过多显存
  const pixelRatio = Math.min(window.devicePixelRatio || 1, 3)
  renderer.setPixelRatio(pixelRatio)
  renderer.setSize(EXPANDED_SIZE, EXPANDED_SIZE, false)
  renderer.setClearColor(0x000000, 0)
  renderer.outputColorSpace = THREE.SRGBColorSpace
  renderer.toneMapping = THREE.ACESFilmicToneMapping
  renderer.toneMappingExposure = 0.8
  renderer.shadowMap.enabled = true
  renderer.shadowMap.type = THREE.PCFSoftShadowMap

  const scene = new THREE.Scene()
  const camera = new THREE.OrthographicCamera(-34, 34, 34, -34, 0.1, 300)
  camera.position.set(0, 0, 100)

  let environment: THREE.Texture | null = null

  const texture = sandblastedTexture()
  // 磨砂金：粗糙度随颗粒起伏，表面是细密的哑光质感
  const matteGold = (color: number, envMapIntensity: number, roughness = 0.72, bumpScale = 0.65) =>
    new THREE.MeshPhysicalMaterial({
      color,
      metalness: 0.7,
      roughness,
      roughnessMap: texture,
      bumpMap: texture,
      bumpScale,
      clearcoat: 0,
      envMapIntensity,
    })
  // 抛光金：低粗糙度，对环境的反射集中成明亮的高光
  const polishedGold = (color: number, envMapIntensity: number) =>
    new THREE.MeshPhysicalMaterial({
      color,
      metalness: 1,
      roughness: 0.12,
      envMapIntensity,
    })

  // 边框：抛光亮面，与磨砂底面形成质感对比
  const frame = polishedGold(isV3.value ? 0xd9a83a : 0xaeb9c7, 0.75)
  // 侧壁与背面：磨砂，略暗
  const side = matteGold(isV3.value ? 0xb07f22 : 0x6f7c8d, 0.6)
  // 底面：独立平面的颗粒强度 0.65，粗糙度 0.85
  const floor = matteGold(isV3.value ? 0xd9a83a : 0xaeb9c7, 0.75, 0.85, 0.65)
  // 凸起的 V3：抛光亮面
  const lettering = polishedGold(isV3.value ? 0xf3cd64 : 0xe4ebf3, 1.5)
  const materials = [frame, side, floor, lettering]

  const group = new THREE.Group()
  // 主光偏正面斜上方：既照亮凹槽内壁，又让凸起的外缘和文字投出阴影
  group.rotation.set(0, 0, 0)
  scene.add(group)

  const geometries: THREE.BufferGeometry[] = []

  // 一整块金属：一次建模出「凸起的窄边框 + 凹下去的喷砂面 + 相连的底部」，是单个连续网格。
  // 材质顺序与几何的分组一一对应，用常量排位，避免顺序错位后材质贴到别的面上
  const medalGeometry = buildMedalGeometry(MEDAL)
  const medalMaterials: THREE.Material[] = []
  medalMaterials[MEDAL_MATERIAL.frame] = frame
  medalMaterials[MEDAL_MATERIAL.wall] = side
  medalMaterials[MEDAL_MATERIAL.floor] = floor
  const medal = new THREE.Mesh(medalGeometry, medalMaterials)
  medal.castShadow = true
  medal.receiveShadow = true
  geometries.push(medalGeometry)
  group.add(medal)

  // 底面单独一张平面：只有它使用 UV 和明显的喷砂纹理。
  // 主体不统一加 UV，避免背面、侧壁因平面投影而出现拉伸或异常颗粒。
  const fieldGeometry = new THREE.ShapeGeometry(badgeShape(MEDAL.frameInner), 64)
  const field = new THREE.Mesh(fieldGeometry, floor)
  field.position.z = MEDAL.floorZ + 0.02
  field.receiveShadow = true
  geometries.push(fieldGeometry)
  group.add(field)

  const font = new FontLoader().parse(fontData)
  const textGeometry = new TextGeometry(text.value, {
    font,
    size: 22,
    depth: 2.0,
    bevelEnabled: true,
    bevelThickness: 0.5,
    bevelSize: 0.35,
    bevelSegments: 5,
    curveSegments: 12,
  })
  textGeometry.computeBoundingBox()
  const bounds = textGeometry.boundingBox
  if (bounds) {
    textGeometry.translate(
      -(bounds.max.x + bounds.min.x) / 2,
      -(bounds.max.y + bounds.min.y) / 2,
      0,
    )
  }
  // 单个数字放大后从底面向上凸起，顶面仍略高于边框
  const textMesh = new THREE.Mesh(textGeometry, lettering)
  textMesh.position.set(0, -0.4, 2.0)
  textMesh.castShadow = true
  geometries.push(textGeometry)
  group.add(textMesh)

  // ── 两盏灯 ──
  // 亮度主要交给上面的环境反射，这两盏灯只是辅助：强度太高会把所有面一起冲亮，
  // 磨砂和抛光的差别、凹凸的明暗就都看不出来了。
  // 光源一：整体光。半球光从上方到下方均匀过渡，没有方向、不聚光，只负责托住暗部不让它发黑
  const fill = new THREE.HemisphereLight(0xfff6e0, 0xc9a35a, 0.9)
  scene.add(fill)

  // 光源二：描边光。从正前偏左上斜着打的平行光，强度适中：勾出边框轮廓与倒角高光，
  // 并让凸起的 V3 投下阴影，但不足以把整个正面照亮。
  // 灯固定在场景里不随徽章旋转：转动时高光与阴影相对变化，立体感在任何角度都成立
  const rim = new THREE.DirectionalLight(0xffffff, 1.3)
  rim.position.set(-30, 24, 40)
  rim.castShadow = true
  rim.shadow.mapSize.set(2048, 2048)
  rim.shadow.camera.left = -35
  rim.shadow.camera.right = 35
  rim.shadow.camera.top = 35
  rim.shadow.camera.bottom = -35
  rim.shadow.camera.near = 1
  rim.shadow.camera.far = 140
  rim.shadow.bias = -0.0003
  rim.shadow.normalBias = 0.02
  scene.add(rim)

  // ── 金属反射用的环境 ──
  // 抛光金属的亮度和高光几乎全来自它反射的环境，而不是被灯直接照亮。
  // 这里自己搭一个亮房间烘焙成环境贴图，四周都有亮度，所以任何朝向的面都不会发黑。
  //
  // 注意：环境要整体保持偏暗。V3 正面、边框顶面、凹面朝向相同，反射到的是同一片环境，
  // 它们之间的亮度差只能来自各自的材质颜色。环境太亮会把所有面一起冲到过曝，颜色差别就没了。
  const studio = new THREE.Scene()
  // 房间底色：中等偏暗的暖灰，是磨砂面的主要亮度来源
  studio.background = new THREE.Color(0.16, 0.14, 0.11)
  function softbox(width: number, height: number, color: number, intensity: number, position: [number, number, number]) {
    const panel = new THREE.Mesh(
      new THREE.PlaneGeometry(width, height),
      new THREE.MeshBasicMaterial({ color: new THREE.Color(color).multiplyScalar(intensity), side: THREE.DoubleSide }),
    )
    panel.position.set(...position)
    // 柔光面始终正对房间中心
    panel.lookAt(0, 0, 0)
    studio.add(panel)
    return panel
  }
  // 正前方：主柔光面，强度适中。抛光面把它反射成一片明亮但不刺眼的高光，
  // 磨砂面把它糊成一层柔和的提亮
  softbox(34, 30, 0xfff6e2, 2.2, [0, 4, 60])
  // 左上：辅助柔光面，给倒角和边框加一层方向性高光
  softbox(22, 28, 0xfff4dc, 1.6, [-34, 30, 34])
  // 右侧：竖长的窄光面，在抛光的倒角上拉出一条细高光
  softbox(6, 40, 0xffffff, 3.0, [44, 4, 20])
  // 底部：弱光面，托住下方的暗部，不让下边缘发黑
  softbox(36, 10, 0xffe2b0, 1.0, [0, -44, 22])

  const pmrem = new THREE.PMREMGenerator(renderer)
  environment = pmrem.fromScene(studio, 0.04).texture
  scene.environment = environment
  pmrem.dispose()
  studio.traverse((object) => {
    if (object instanceof THREE.Mesh) {
      object.geometry.dispose()
      ;(object.material as THREE.Material).dispose()
    }
  })

  // ── 鼠标拖拽旋转 ──
  // 目标角度与当前角度分离，松开后平滑回正到正面
  const targetRotation = new THREE.Euler(0, 0, 0)
  let dragging = false
  let lastPointer: { x: number; y: number } | null = null
  // 按下时的坐标，用于判断这次操作是点击还是拖拽
  let dragOrigin: { x: number; y: number } | null = null
  let animationFrame = 0

  function draw() {
    // 进出场旋转优先：直接按时间设定角度，不让「平滑趋近」把它拉回正面
    const angle = spinAngle(performance.now())
    if (angle !== null) {
      group.rotation.set(0, angle, 0)
      targetRotation.set(0, angle, 0)
    } else {
      group.rotation.x += (targetRotation.x - group.rotation.x) * 0.18
      group.rotation.y += (targetRotation.y - group.rotation.y) * 0.18
      group.rotation.z += (targetRotation.z - group.rotation.z) * 0.18
    }
    renderer.render(scene, camera)
  }

  function animate() {
    draw()
    const settled =
      !spin &&
      Math.abs(targetRotation.x - group.rotation.x) < 0.001 &&
      Math.abs(targetRotation.y - group.rotation.y) < 0.001 &&
      Math.abs(targetRotation.z - group.rotation.z) < 0.001
    // 拖动中、旋转中或尚未稳定时继续；静止后停止重绘
    animationFrame = dragging || !settled ? requestAnimationFrame(animate) : 0
  }

  function wake() {
    if (!animationFrame) animationFrame = requestAnimationFrame(animate)
  }

  // ── 打开 / 关闭时的 3D 旋转 ──
  // 让徽章模型自己沿竖轴转一整圈，而不是转动画布这个矩形元素。
  // 这样数字、边框、侧面厚度都跟着徽章一起转，才是真正的 3D 旋转。
  let spin: { start: number; duration: number; from: number; to: number } | null = null

  /** 当前时刻模型应处的旋转角度；返回 null 表示不在旋转中 */
  function spinAngle(now: number): number | null {
    if (!spin) return null
    const { start, duration, from, to } = spin
    const progress = Math.min(1, (now - start) / duration)
    // 缓入缓出，和位置、缩放的移动节奏一致
    const eased = progress < 0.5 ? 2 * progress * progress : 1 - (-2 * progress + 2) ** 2 / 2
    // 转满一圈后结束；最后一帧的角度正好是终点值
    if (progress >= 1) spin = null
    return from + (to - from) * eased
  }

  /** 开始一次进出场旋转：从 from 转到 to（弧度），历时 duration 毫秒 */
  function startSpin(from: number, to: number, duration: number) {
    spin = { start: performance.now(), duration, from, to }
    wake()
  }

  // 画布默认是空的，必须在初始化后主动画一帧，否则页面刚打开时徽章位置是一片空白，
  // 要等到第一次交互触发重绘才会出现
  renderer.render(scene, camera)

  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0) return
    dragging = true
    wasDragged = false
    // 记录按下点：之后用「离按下点的总位移」判断是否拖拽，
    // 而不是单个移动事件的位移（真实鼠标每个事件常只移动 1~2px）
    dragOrigin = { x: event.clientX, y: event.clientY }
    lastPointer = { x: event.clientX, y: event.clientY }
    targetRotation.set(group.rotation.x, group.rotation.y, group.rotation.z)
    target.setPointerCapture(event.pointerId)
    wake()
  }

  function onPointerMove(event: PointerEvent) {
    if (!dragging || !lastPointer) return
    const dx = event.clientX - lastPointer.x
    const dy = event.clientY - lastPointer.y
    if (dragOrigin) {
      const totalX = event.clientX - dragOrigin.x
      const totalY = event.clientY - dragOrigin.y
      // 离按下点超过几像素就算拖拽，松手时不再触发点击
      if (Math.hypot(totalX, totalY) > 4) wasDragged = true
    }
    lastPointer = { x: event.clientX, y: event.clientY }
    targetRotation.y += dx * 0.016
    targetRotation.x += dy * 0.016
    // 上下翻转限制在接近侧视的角度，避免完全翻到背面
    targetRotation.x = Math.max(-1.1, Math.min(1.1, targetRotation.x))
    wake()
  }

  function endDrag(event: PointerEvent) {
    if (!dragging) return
    dragging = false
    lastPointer = null
    dragOrigin = null
    if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId)
    // 松开后回正到正面
    targetRotation.set(0, 0, 0)
    wake()
  }

  target.addEventListener('pointerdown', onPointerDown)
  target.addEventListener('pointermove', onPointerMove)
  target.addEventListener('pointerup', endDrag)
  target.addEventListener('pointercancel', endDrag)

  disposeBadge = () => {
    if (animationFrame) cancelAnimationFrame(animationFrame)
    target.removeEventListener('pointerdown', onPointerDown)
    target.removeEventListener('pointermove', onPointerMove)
    target.removeEventListener('pointerup', endDrag)
    target.removeEventListener('pointercancel', endDrag)
    geometries.forEach((geometry) => geometry.dispose())
    materials.forEach((material) => material.dispose())
    texture.dispose()
    environment?.dispose()
    renderer.dispose()
    resetView = null
  }

  // 切换放大状态时调用：让模型转一整圈。
  // opening 为 true 是打开（正向转一圈），false 是关闭（反向转一圈）。
  // 旋转时长与样式里 medal-flip-in / medal-flip-out 的时长保持一致
  resetView = (opening: boolean) => {
    // 从正面出发转一整圈后回到正面，所以起止都是正面，不会停在侧面
    const FULL_TURN = Math.PI * 2
    startSpin(0, opening ? FULL_TURN : -FULL_TURN, opening ? 800 : 550)
  }
}

/** 本次按压是否发生了拖拽：用于区分「点击放大」和「拖拽旋转」。
 *  它不需要驱动界面更新，用普通变量即可，避免任何响应式同步时机带来的歧义 */
let wasDragged = false
/** 由 initBadge 提供：复位缩放并让模型转一整圈（true = 打开，false = 关闭） */
let resetView: ((opening: boolean) => void) | null = null

/** 小勋章中心相对屏幕中心的偏移（px）和它相对放大后的缩放比，写成 CSS 变量供动画使用。
 *  这样打开时从页面上那枚小勋章的真实位置出发，关闭时也飞回同一个位置 */
const originX = ref(0)
const originY = ref(0)
const originScale = ref(1)

function toggleExpanded() {
  // 本次是拖拽则不触发点击放大；标志用完即重置，避免影响下一次点击
  const wasDrag = wasDragged
  wasDragged = false
  if (wasDrag) return
  if (expanded.value) close()
  else open()
}

function measureOrigin() {
  const el = root.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  // 放大效果来自按钮上单独的 scale 属性，transform 里的平移和缩放都会被它再乘一次。
  // 所以偏移量要先除以这个倍数，起点缩放用它的倒数，这样第一帧正好落在小勋章的位置和大小
  const k = EXPANDED_SCALE
  originX.value = (rect.left + rect.width / 2 - window.innerWidth / 2) / k
  originY.value = (rect.top + rect.height / 2 - window.innerHeight / 2) / k
  originScale.value = 1 / k
}

function open() {
  // 必须在传送到浮层之前量，此时徽章还在文档流里的原位
  measureOrigin()
  closing.value = false
  expanded.value = true
  resetView?.(true)
}

/** 收起：先播放回程动画，等浏览器通知动画真正结束，才把徽章放回文档流。
 *  此时徽章在浮层里，量不到文档流原位，所以沿用打开时记录的位置 */
function close() {
  if (!expanded.value || closing.value) return
  closing.value = true
  // 关闭：模型反向转一整圈，同时沿原路缩小回到小勋章的位置
  resetView?.(false)
}

/** 回程动画结束的回调。不用定时器：页面卡顿时动画会被拖慢，定时器却照走，动画就被截断了。
 *  scoped 样式会给动画名加哈希后缀（如 medal-flip-out-bac476d6），所以用「以此开头」来匹配 */
function onFlipOutEnd(event: AnimationEvent) {
  if (!event.animationName.startsWith('medal-flip-out') || !closing.value) return
  expanded.value = false
  closing.value = false
}

onMounted(() => {
  window.addEventListener('keydown', closeOnEscape)
  try {
    // canvas 设了 pointer-events: none，收不到指针事件；拖拽监听必须挂在外层按钮上
    if (canvas.value && root.value) initBadge(canvas.value, root.value)
  } catch (error) {
    console.warn('3D 徽章初始化失败，使用文字回退', error)
    failed.value = true
  }
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', closeOnEscape)
  disposeBadge?.()
})
</script>

<template>
  <Teleport to="body">
    <Transition name="medal-fade">
      <div v-if="expanded" class="version-medal-backdrop" @click="close">
        <span class="version-medal-hint">拖拽旋转 · 点击空白处或按 Esc 收起</span>
      </div>
    </Transition>
  </Teleport>
  <!-- 按钮始终留在文档流里、保持小尺寸。放大时只把它整体传送到居中的浮层里，
       按钮自身的位置和尺寸不变，所以不会有位移突变，也不会把所在的那一行撑开。
       尺寸和缩放倍数以 CSS 变量传给样式，和脚本里的常量是同一份数值 -->
  <Teleport to="body" :disabled="!expanded">
    <div
      :class="['version-medal-stage', { 'is-open': expanded, 'is-closing': closing }]"
      :style="{
        '--origin-x': originX + 'px',
        '--origin-y': originY + 'px',
        '--origin-scale': originScale,
        '--small-size': SMALL_SIZE + 'px',
        '--expanded-size': EXPANDED_SIZE + 'px',
        '--expanded-scale': EXPANDED_SCALE,
      }"
    >
      <button
        type="button"
        ref="root"
        :class="['version-medal', isV3 ? 'version-v3' : 'version-v2', { 'is-expanded': expanded }]"
        :title="expanded ? '拖拽旋转，点击空白处收起' : `${description}，点击放大查看`"
        :aria-label="expanded ? '收起徽章' : `${description}，点击放大查看`"
        :aria-expanded="expanded"
        @click="toggleExpanded"
        @animationend="onFlipOutEnd"
      >
        <canvas v-show="!failed" ref="canvas" aria-hidden="true"></canvas>
        <span v-if="failed" class="version-fallback">{{ text }}</span>
      </button>
    </div>
  </Teleport>
</template>

<style scoped>
/* 浮层：收起时只是文档流里的一个占位；放大时传送到 body 并固定居中。
   占位高度固定为所在行的自然行高，不随勋章大小变化，所以调大勋章也不会把那一行撑高。
   勋章在占位里居中，比行高多出的部分上下均匀溢出，不影响布局。
   margin-left: auto 让它在所在的弹性行里靠最右侧，前面的内容保持原位 */
.version-medal-stage {
  --row-height: 25px;
  display: inline-flex;
  width: var(--small-size);
  height: var(--row-height);
  flex: none;
  margin-left: auto;
  transform-origin: center;
}

.version-medal-stage.is-open {
  position: fixed;
  top: 50%;
  left: 50%;
  z-index: 1001;
  width: var(--expanded-size);
  height: var(--expanded-size);
  align-items: center;
  justify-content: center;
  /* 负边距取宽高的一半，让浮层正好居中 */
  margin: calc(var(--expanded-size) / -2) 0 0 calc(var(--expanded-size) / -2);
}

/* 打开与关闭只负责「位置」和「大小」：从页面上小勋章的位置出发，放大并移动到屏幕中央。
   旋转不在这里做 —— 转动画布这个矩形只是把整张图压扁再展开，里面的数字不会跟着转。
   真正的 3D 旋转由模型自己完成（见 startSpin），数字、边框、侧面都会一起转。
   起点的偏移和缩放由 originX / originY / originScale 三个变量给出（见 measureOrigin）。
   动画名和时长不能随意改：JS 靠 medal-flip-out 的结束事件来收起徽章，时长要与 startSpin 保持一致 */
.version-medal-stage.is-open:not(.is-closing) .version-medal {
  animation: medal-flip-in 0.8s cubic-bezier(0.33, 0, 0.2, 1) both;
}

.version-medal-stage.is-open.is-closing .version-medal {
  animation: medal-flip-out 0.55s cubic-bezier(0.55, 0, 0.55, 0.2) both;
}

/* 徽章在浮层里本来就居中（屏幕中心），所以只需从那里平移到小勋章的位置：
   偏移量就是小勋章中心相对屏幕中心的距离 */
@keyframes medal-flip-in {
  from {
    transform: translate(var(--origin-x), var(--origin-y)) scale(var(--origin-scale));
  }
  to {
    transform: translate(0, 0) scale(1);
  }
}

/* 关闭：沿原路缩小并移回小勋章的位置 */
@keyframes medal-flip-out {
  from {
    transform: translate(0, 0) scale(1);
  }
  to {
    transform: translate(var(--origin-x), var(--origin-y)) scale(var(--origin-scale));
  }
}

.version-medal {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 0;
  appearance: none;
  background: transparent;
  cursor: zoom-in;
  width: var(--small-size);
  height: var(--small-size);
  flex: none;
  filter: drop-shadow(0 2px 1.5px rgba(57, 32, 8, 0.32)) drop-shadow(0 0 2px rgba(212, 151, 45, 0.2));
  transition: transform 0.2s ease, filter 0.2s ease;
  backface-visibility: visible;
}

.version-medal.version-v2 {
  filter: drop-shadow(0 2px 1.5px rgba(15, 23, 42, 0.3)) drop-shadow(0 0 2px rgba(100, 116, 139, 0.2));
}

.version-medal canvas {
  display: block;
  width: 100%;
  height: 100%;
  pointer-events: none;
}

/* 放大状态靠缩放实现，宽高保持小尺寸不变 */
.version-medal.is-expanded,
.version-medal.is-expanded:hover {
  position: static;
  width: var(--small-size);
  height: var(--small-size);
  cursor: grab;
  touch-action: none;
  transform: none;
  filter: drop-shadow(0 16px 14px rgba(0, 0, 0, 0.45)) drop-shadow(0 0 16px rgba(212, 151, 45, 0.35));
  scale: var(--expanded-scale);
}

.version-medal.is-expanded:active {
  cursor: grabbing;
}

.version-fallback {
  color: #b47a1c;
  font-size: 10px;
  font-weight: 800;
}

.is-expanded .version-fallback {
  color: #f3d27a;
  font-size: 72px;
}

.version-medal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  background: rgba(15, 23, 42, 0.72);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
}

.version-medal-hint {
  position: absolute;
  bottom: 36px;
  left: 50%;
  transform: translateX(-50%);
  color: rgba(255, 255, 255, 0.72);
  font-size: 12px;
  white-space: nowrap;
  pointer-events: none;
}

.medal-fade-enter-active,
.medal-fade-leave-active {
  transition: opacity 0.2s ease;
}

.medal-fade-enter-from,
.medal-fade-leave-to {
  opacity: 0;
}
</style>
