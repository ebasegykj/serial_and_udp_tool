r'''
Copyright (C) 2026 Wu Ruiqing. All rights reserved.
HIS WORK CONTAINS TRADE SECRET AND PROPRIETARY INFORMATION WHICH IS THE PROPERTY OF 
SCLQ.Co.Ltd. OR ITS LICENSORS AND IS SUBJECT TO LICENSE TERMS. 
Author: gykjwrq@163.com
Date: 2026-09-15 12:06:01
Version: 1.2
LastEditors: wrq
LastEditTime: 2026-11-03
FilePath: \serial_udp\14多语言折叠版.py
description: 
Revision: 1.1 修复：跨线程读Tkinter变量改为配置快照；原始视图改用帧序号增量刷新；
          串口扫描改为可选深度扫描并用with管理注册表句柄；接收线程异常退出时主线程提示；
          帧计数拆分为帧/样本两级；通道索引越界校验；提取CSV保存代码去重；一阶差分增量计算。
Revision: 1.2 2026-11-03 修复A：显式设置 AppUserModelID（任务栏按其分组取图标）+
          图标路径多候选鲁棒加载（Nuitka目录/脚本目录/qt5目录，缺失则跳过不报错）；
          修复B：win32 高DPI感知（创建root前调用）+ 窗口尺寸/最小尺寸按屏幕工作区自适应，
          避免低分辨率电脑上 1100x900 窗口与 900x700 minsize 超出屏幕。
Revision: 1.2 2026-11-03 14版新增：①Connection/Data Log/Frame Configuration 三个分组可折叠
          （右上角箭头按钮切换，grid_remove/按原参数恢复）；②启动按屏幕工作区高度自适应
          默认折叠（work_h<820 时 Data Log 与 Frame Configuration 默认收起）；
          ③ui_config.json 持久化折叠状态与语言（Nuitka/脚本目录，切换即写回）；
          ④中英文切换（主要控件，"中/EN"按钮，默认英文与13版观感一致）。
Revision: 1.2 2026-11-03 14版调整：①折叠箭头并入分组标题文字（"Connection ▼/▶"），
          删除右上角浮动箭头按钮，点击标题行（y<=16）切换折叠；
          ②中/EN 按钮移位：紧跟 Interface 下拉框（col2），Help 与 🌙 推到行尾。
Revision: 1.2 2026-11-03 14版重构：折叠方案改为 body 容器包裹——每组在 labelframe 内
          建 body(tk.Frame)，组内控件全部为 body 子控件，折叠/展开仅操作 body 的
          grid_remove/grid。修复两个实测 bug：a) 启动时 Connection/Frame Configuration
          整组消失（旧快照方案与 serial/udp 同格叠放及时序冲突）；b) 折叠后组框残留
          ~110px 空壳不缩行。组内 columnconfigure 同步移至 body。
Revision: 1.2 2026-11-03 布局紧凑化：主题按钮 🌙 emoji 改文字按钮 Theme/主题（纳入 i18n）；
          Frame Configuration 标签缩短（Smp:/Byte Ord:/Max Pts:，zh 同步）；
          Data Log 文件名标签 wraplength 150→96；Refresh/Deep Scan 按钮 width 8→7/9→8。
Revision: 1.2 2026-11-03 布局二改（按用户截图逐条）：①Connection 首行改 pack 容器，
          Help/中EN/Theme 修复回行尾显示（网格共列导致被挤出可视区）；②Deep Scan 按钮删除，
          只留一个 Refresh，宽度让给端口下拉框；③Load File / Analyze Clipboard 移到
          Frame Configuration 组（原 Apply Frame 行）；④Apply Frame 上移至 Max Pts 行；
          ⑤Clear Plot 移到右侧 Send 标题行最右边；⑥启动一律全部展开，不再按屏幕高度
          自动折叠、不恢复上次折叠状态（折叠仅作手动操作）；⑦分组标题
          Frame Configuration → Frame Config / Offline（中：帧配置/离线绘图）。
'''
#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Binary Frame Analyzer - 解析串口二进制帧数据并绘图
支持按帧头同步、指定字节索引提取数值、实时/离线模式
支持多路数据同时绘图、每路独立符号位配置、一阶差分
支持 5 套主题切换：深色/原始/淡绿/淡蓝/米黄
支持中英文界面切换（主要控件）、三个设置分组可折叠
"""
import re
import sys
import json
import tkinter as tk
from tkinter import ttk, scrolledtext, messagebox, filedialog
import serial
import serial.tools.list_ports
import socket
import threading
import time
import os
from matplotlib.backends.backend_tkagg import FigureCanvasTkAgg, NavigationToolbar2Tk
from matplotlib.figure import Figure
from collections import deque

# ==========================================================
# 5 套主题配色
# ==========================================================
# 深色（启动默认）
DARK_THEME = {
    'bg':       '#13141c', 'panel':    '#252938',
    'panel_2':  '#2d3244', 'border':   '#3a4056',
    'text':     '#ffffff', 'text_dim': '#9da3bb',
    'accent':   '#8b9bff', 'accent_2': '#5ce0d0',
    'plot_bg':  '#0e0f16', 'plot_grid':'#2a2f42',
    'titlebar': '#252938',
}

# 原始（Tkinter 默认灰白）
NATIVE_THEME = {
    'bg':       '#d9d9d9', 'panel':    '#f0f0f0',
    'panel_2':  '#ffffff', 'border':   '#a0a0a0',
    'text':     '#000000', 'text_dim': '#606060',
    'accent':   '#0078d4', 'accent_2': '#106ebe',
    'plot_bg':  '#ffffff', 'plot_grid':'#d0d0d0',
    'titlebar': '#f0f0f0',
}

# 淡绿
GREEN_THEME = {
    'bg':       '#f0f7f2', 'panel':    '#ffffff',
    'panel_2':  '#e3f0e7', 'border':   '#b8d8c1',
    'text':     '#1a2e1f', 'text_dim': '#5a7a63',
    'accent':   '#2d8a4e', 'accent_2': '#4ab97a',
    'plot_bg':  '#fafdfb', 'plot_grid':'#d8e8dd',
    'titlebar': '#ffffff',
}

# 淡蓝
BLUE_THEME = {
    'bg':       '#eef4fb', 'panel':    '#ffffff',
    'panel_2':  '#dce8f5', 'border':   '#aec8e0',
    'text':     '#1a2233', 'text_dim': '#5a6b85',
    'accent':   '#2c6bb5', 'accent_2': '#4a9de0',
    'plot_bg':  '#fafcff', 'plot_grid':'#d4e2f2',
    'titlebar': '#ffffff',
}

# 米黄
YELLOW_THEME = {
    'bg':       '#faf6ec', 'panel':    '#ffffff',
    'panel_2':  '#f2ecdb', 'border':   '#d4c9a8',
    'text':     '#2e2416', 'text_dim': '#7a6a4a',
    'accent':   '#b8860b', 'accent_2': '#d4a234',
    'plot_bg':  '#fffdf7', 'plot_grid':'#e8dfc6',
    'titlebar': '#ffffff',
}
PINK_THEME = {
    'bg':       '#fdf2f8', 'panel':    '#ffffff',
    'panel_2':  '#fce7f3', 'border':   '#f9a8d4',
    'text':     '#4a1d3a', 'text_dim': '#9d5c82',
    'accent':   '#db2777', 'accent_2': '#ec4899',
    'plot_bg':  '#fff7fb', 'plot_grid':'#f9d4e8',
    'titlebar': '#ffffff',
}
MAC_THEME = {
    'bg':       '#e0e0e6',   # 主背景：中灰（比 Win11 稍深）
    'panel':    '#f5f5f7',   # 卡片：macOS 经典浅灰
    'panel_2':  '#ffffff',   # 输入框：纯白
    'border':   '#c8c8d0',   # 边框：中灰
    'text':     '#1d1d1f',   # 主文字：macOS 近黑
    'text_dim': '#6e6e73',   # 次要文字：macOS 灰
    'accent':   '#007aff',   # macOS 蓝（经典）
    'accent_2': '#0a84ff',   # hover 亮蓝
    'plot_bg':  '#fbfbfd',   # 绘图区：近白
    'plot_grid': '#e5e5ea',  # 网格：浅灰（macOS 标准）
    'titlebar': '#f5f5f7',   # 标题栏：跟 panel 一致
}
WIN_THEME = {
    'bg':       '#e8e8ed',   # 主背景：中浅灰（明显比 panel 深）
    'panel':    '#f7f7fa',   # 卡片：浅灰白
    'panel_2':  '#ffffff',   # 输入框：纯白（最亮）
    'border':   '#d0d0d8',   # 边框：中灰
    'text':     '#1a1a1f',   # 主文字：近黑
    'text_dim': '#6e6e78',   # 次要文字：中灰
    'accent':   '#0067c0',   # Win11 蓝
    'accent_2': '#1976d2',   # hover 蓝
    'plot_bg':  '#fbfbfd',   # 绘图区：近白
    'plot_grid': '#e6e6ec',  # 网格：浅灰
    'titlebar': '#f7f7fa',   # 标题栏：跟 panel 一致
}

# 主题列表（顺序：深色 → 原始 → 淡绿 → 淡蓝 → 米黄）
THEME_LIST = [
    ('深色', DARK_THEME,    '🌙'),
    ('原始', NATIVE_THEME,  '⬜'),
    ('淡绿', GREEN_THEME,   '🍃'),
    ('淡蓝', BLUE_THEME,    '💧'),
    ('米黄', YELLOW_THEME,  '☀'),
    ('粉色', PINK_THEME,    '🌸'),   # ← 新增
    ('Mac',  MAC_THEME,     '🍎'),   # ← 新增
    ('Win',  WIN_THEME,     'Win'),   # ← 新增
]

# 全局主题变量（启动时用深色，即 index=0）
THEME = dict(DARK_THEME)

# 通道颜色
CHANNEL_COLORS = [
    '#4dd4c4', '#7c8cff', '#ff6b81', '#ffd166',
    '#a78bfa', '#4ade80', '#f472b6', '#94a3b8',
]
MAX_CHANNELS = 8

# 全局字体
FONT_UI = ('Microsoft YaHei UI', 9)
FONT_LABEL = ('Microsoft YaHei UI', 9)
FONT_TITLE = ('Microsoft YaHei UI', 10, 'bold')
FONT_MONO = ('Consolas', 10)
FONT_SMALL = ('Microsoft YaHei UI', 8)


# ==========================================================
# 1.2 新增（功能4）：中英文词典（仅主要控件）
# zh = 中文界面；en = 英文界面（默认，与13版观感一致）
# 注意：LabelFrame 标题保留前后双空格与原版式一致
# ==========================================================
TEXTS = {
    'en': {
        # 分组标题
        'connection':      '  Connection  ',
        'datalog':         '  Data Log  ',
        'framecfg':        '  Frame Config / Offline  ',
        'chancfg':         '  Channel Configuration  ',
        # 按钮
        'help':            'Help',
        'refresh':         'Refresh',
        'deep_scan':       'Deep Scan',
        'connect':         'Connect',
        'disconnect':      'Disconnect',
        'load_file':       'Plot Raw Data',
        'analyze_clip':    'Plot from Clipboard',
        'save_extracted':  'Save Samples',
        'auto_save':       'Save Raw Frames',
        'csv_file':        'CSV File...',
        'save_file':       'TXT File...',
        'apply_frame':     'Apply Frame',
        'clear_plot':      'Clear Plot',
        'send':            'Send',
        'history':         'History',
        'plot_title':      '  Data Plot  ',
        'view_plot':       'Plot',
        'view_raw':        'Raw',
        'apply':           'Apply',
        'lang_btn':        '中',          # 当前英文 → 显示"中"提示可切中文
        'theme_btn':       'Theme',       # 主题切换（文字按钮，替代 emoji 避免豆腐块）
        # 标签
        'interface':       'Interface:',
        'port':            'Port:',
        'baud':            'Baud:',
        'local_ip':        'Local IP:',
        'local_port':      'Local Port:',
        'remote_ip':       'Remote IP:',
        'remote_port':     'Remote Port:',
        'header':          'Header:',
        'length':          'Length:',
        'byte_order':      'Byte Ord:',
        'samples_pkt':     'Smp:',
        'max_points':      'Max Pts:',
        'channels':        'Channels:',
    },
    'zh': {
        # 分组标题
        'connection':      '  数据连接  ',
        'datalog':         '  数据记录  ',
        'framecfg':        '  帧配置/离线绘图  ',
        'chancfg':         '  通道配置  ',
        # 按钮
        'help':            '帮助',
        'refresh':         '刷新',
        'deep_scan':       '深度扫描',
        'connect':         '连接',
        'disconnect':      '断开',
        'load_file':       '原始数据绘图',
        'analyze_clip':    '剪贴板数据绘图',
        'save_extracted':  '提取数据保存',
        'auto_save':       '原始数据保存',
        'csv_file':        'CSV文件...',
        'save_file':       'TXT文件...',
        'apply_frame':     '应用帧配置',
        'clear_plot':      '清空绘图',
        'send':            '发送',
        'history':         '历史发送',
        'plot_title':      '  数据绘图  ',
        'view_plot':       '绘图',
        'view_raw':        '原始',
        'apply':           '应用',
        'lang_btn':        'EN',          # 当前中文 → 显示"EN"提示可切英文
        'theme_btn':       '主题',
        # 标签
        'interface':       '接口:',
        'port':            '端口:',
        'baud':            '波特率:',
        'local_ip':        '本机IP:',
        'local_port':      '本机端口:',
        'remote_ip':       '远端IP:',
        'remote_port':     '远端端口:',
        'header':          '帧头:',
        'length':          '帧长:',
        'byte_order':      '字节序:',
        'samples_pkt':     '样本:',
        'max_points':      '点数:',
        'channels':        '通道数:',
    },
}


# ==========================================================
# 2026-11-03 修复B①：Windows 高DPI感知
# 必须在创建 Tk root 之前调用（见文件末尾 main 段），
# 否则在开启 DPI 缩放/低分辨率电脑上窗口会被系统拉伸发虚、
# 且写死像素尺寸容易超出屏幕。
# ==========================================================
def setup_windows_dpi_awareness():
    if sys.platform != 'win32':
        return
    try:
        import ctypes
        try:
            ctypes.windll.shcore.SetProcessDpiAwareness(1)
        except Exception:
            try:
                ctypes.windll.user32.SetProcessDPIAware()
            except Exception:
                pass
    except Exception:
        pass


def apply_dark_titlebar(root, color_hex='#252938'):
    """把 Windows 标题栏改成指定颜色（Win10 1809+ / Win11 有效）"""
    if sys.platform != 'win32':
        return
    try:
        import ctypes
        root.update_idletasks()
        hwnd = ctypes.windll.user32.GetParent(root.winfo_id())
        if not hwnd:
            hwnd = root.winfo_id()

        r = int(color_hex[1:3], 16)
        g = int(color_hex[3:5], 16)
        b = int(color_hex[5:7], 16)
        is_dark = (r + g + b) < 384

        DWMWA_USE_IMMERSIVE_DARK_MODE = 20
        value = ctypes.c_int(1 if is_dark else 0)
        ctypes.windll.dwmapi.DwmSetWindowAttribute(
            hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE,
            ctypes.byref(value), ctypes.sizeof(value)
        )

        DWMWA_CAPTION_COLOR = 35
        colorref = (b << 16) | (g << 8) | r
        color_value = ctypes.c_int(colorref)
        ctypes.windll.dwmapi.DwmSetWindowAttribute(
            hwnd, DWMWA_CAPTION_COLOR,
            ctypes.byref(color_value), ctypes.sizeof(color_value)
        )

        DWMWA_TEXT_COLOR = 36
        if is_dark:
            tr, tg, tb = 0xe4, 0xe6, 0xf0
        else:
            tr, tg, tb = 0x1a, 0x1d, 0x2a
        text_colorref = (tb << 16) | (tg << 8) | tr
        text_value = ctypes.c_int(text_colorref)
        ctypes.windll.dwmapi.DwmSetWindowAttribute(
            hwnd, DWMWA_TEXT_COLOR,
            ctypes.byref(text_value), ctypes.sizeof(text_value)
        )
    except Exception:
        pass


class FrameAnalyzer:
    def __init__(self, root):
        self.root = root

        # 2026-11-03 修复A：显式设置 AppUserModelID，root 创建后、iconbitmap 之前调用
        self._set_app_user_model_id()

        # 当前主题索引（0 = 深色）
        self.theme_index = 0

        def get_icon_path():
            # 2026-11-03 用户要求恢复旧版行为：把任意 app.ico 放到 exe 同目录，
            # 软件就用这个图标。因此 exe 同目录优先级最高（用户自定义覆盖），
            # 其次是打包内置（onefile 的临时解包目录），再往下才是脚本/开发目录兜底。
            candidates = []
            # 1) exe 所在目录（用户自定义图标，优先级最高）
            try:
                candidates.append(os.path.join(os.path.dirname(os.path.abspath(sys.executable)), "app.ico"))
            except Exception:
                pass
            # 2) Nuitka 打包后二进制所在目录（onefile 为临时解包目录，随包内置）
            base = getattr(sys, "__nuitka_binary_dir__", None)
            if base:
                candidates.append(os.path.join(base, "app.ico"))
            # 3) 脚本所在目录（开发/源码运行）
            try:
                candidates.append(os.path.join(os.path.dirname(os.path.abspath(__file__)), "app.ico"))
            except Exception:
                pass
            # 4) qt5 目录兜底（部分环境下该目录存在 app.ico）
            try:
                script_dir = os.path.dirname(os.path.abspath(__file__))
                candidates.append(os.path.join(script_dir, "qt5", "app.ico"))
                qtdir = os.environ.get("QTDIR", "")
                if qtdir:
                    candidates.append(os.path.join(qtdir, "app.ico"))
            except Exception:
                pass
            for cand in candidates:
                try:
                    if cand and os.path.isfile(cand):
                        return cand
                except Exception:
                    pass
            return None

        icon_path = get_icon_path()
        # 1.2 补充：记住图标路径，供子窗口（帮助/发送历史弹窗）也用同一图标，
        # 否则 Toplevel 默认显示 Tk 羽毛图标
        self._icon_path = icon_path
        if icon_path:
            try:
                self.root.iconbitmap(icon_path)
            except Exception:
                pass

        self._apply_dark_theme()

        self.root.title("Binary Frame Analyzer by gykjwrq@163.com ")
        # 2026-11-03 修复B②③：窗口尺寸与最小尺寸按屏幕工作区自适应
        # （内部同时把工作区尺寸存入 self.work_w / self.work_h，供 1.2 功能2 使用）
        self._setup_dpi_and_geometry()
        self.root.configure(bg=THEME['bg'])

        try:
            apply_dark_titlebar(self.root, THEME['titlebar'])
        except Exception:
            pass

        # ==========================================================
        # 1.2 新增（功能2/3/4）：i18n 状态、折叠状态与配置持久化初始化
        # 2026-11-03 用户要求：启动时一律全部展开，不再按屏幕高度自动折叠，
        # 也不再从 ui_config.json 恢复折叠状态（折叠仅保留手动点击，默认不藏内容）
        # ==========================================================
        self._i18n_widgets = []          # [(widget, key, attr), ...]
        self._collapsible = {}           # key -> {'frame': labelframe, 'body': body}
        self.language = 'zh'             # 2026-11-03 用户要求：默认中文（ui_config.json 有记录则用记录值）
        self.collapsed_state = {
            'connection': False,
            'datalog':    False,
            'framecfg':   False,
        }
        self._config_path = self._get_config_path()
        self._load_ui_config()

        # Serial
        self.serial_port = None
        self.is_connected = False
        self.receive_thread = None
        self.stop_event = threading.Event()
        self.rx_buffer = bytearray()

        # Frame config
        self.frame_header = bytes.fromhex("EB90")
        self.frame_length = 11
        self.byte_order = "big"
        self.samples_per_packet = 1

        # UDP settings
        self.interface_type = "serial"
        self.udp_local_port = 8080
        self.udp_remote_ip = ""
        self.udp_remote_port = 0
        self.udp_socket = None
        self.udp_remote_addr = None

        # Plot config
        self.max_points = 500

        # Channel config
        self.channel_count = 1
        self.channels = []
        self.channel_values = []
        self.channel_diff_values = []
        self.channel_lines = []
        self.channel_diff_lines = []
        self.frame_count = 0    # 原始帧级别计数：每成功解析一帧 +1（用于原始视图标注、状态栏 Frames）
        self.sample_count = 0   # 样本级别计数：samples_per_packet>1 时每帧内每个样本 +1（用于 CSV 提取数据的 Frame 列）

        # 通道配置快照：接收线程只能读这个快照，禁止跨线程读 StringVar/BooleanVar（修复1）
        self.active_channel_configs = []

        # Channel widget refs
        self.channel_widgets = []
        self.channels_container = None

        # Raw data log
        self.auto_save_enabled = False
        self.auto_save_file = ""
        self.rx_total_bytes = 0
        self._save_pending = bytearray()

        # Extracted data save
        self.extracted_save_enabled = False
        self.extracted_save_file = ""
        self._extracted_save_buffer = []
        self._extracted_save_frame_start = 0
        self._extracted_header_written = False

        # View switch
        self.view_mode_var = None
        self.raw_frame_buffer = deque(maxlen=1000)
        self._raw_update_pending = False
        self._raw_last_displayed_seq = None  # 原始视图已显示的最大帧序号（修复2：用 seq 追踪增量）

        self._init_channels(1)

        self.create_widgets()
        self.refresh_ports()
        self._on_interface_change()

        # 1.2 新增：确保语言与折叠状态在控件创建后正确呈现（功能1/4）
        self._apply_language()
        for key in ('connection', 'datalog', 'framecfg'):
            self._set_group_collapsed(key, self.collapsed_state.get(key, False))

        # 1.2 新增：退出时保存一次配置（可选保存点）
        try:
            self.root.protocol("WM_DELETE_WINDOW", self._on_close)
        except Exception:
            pass

    # ==========================================================
    # 2026-11-03 修复A：AppUserModelID（任务栏分组图标）
    # ==========================================================
    def _set_app_user_model_id(self):
        """Windows 任务栏按 AppUserModelID 分组并取关联图标。
        未显式设置时（尤其从 python 直接运行或某些打包形态下），
        任务栏图标可能回落到默认图标。需先初始化 COM。失败静默。"""
        if sys.platform != 'win32':
            return
        try:
            import ctypes
            try:
                ctypes.windll.ole32.CoInitialize(None)
            except Exception:
                pass
            ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID(
                "SCLQ.FrameAnalyzer.14")
        except Exception:
            pass

    # ==========================================================
    # 2026-11-03 修复B②③：窗口尺寸/最小尺寸自适应屏幕工作区
    # ==========================================================
    def _setup_dpi_and_geometry(self):
        """优先用 SPI_GETWORKAREA(132) 读取工作区（不含任务栏），
        失败则回退整屏尺寸。保证 1100x900 / minsize 900x700 在小屏上完整可见。
        1.2 新增：把工作区尺寸存入 self.work_w / self.work_h（功能2 使用）。"""
        work_w = work_h = None
        if sys.platform == 'win32':
            try:
                import ctypes
                import ctypes.wintypes
                rect = ctypes.wintypes.RECT()
                SPI_GETWORKAREA = 132
                if ctypes.windll.user32.SystemParametersInfoW(
                        SPI_GETWORKAREA, 0, ctypes.byref(rect), 0):
                    work_w = rect.right - rect.left
                    work_h = rect.bottom - rect.top
            except Exception:
                work_w = work_h = None
        if not work_w or not work_h:
            try:
                self.root.update_idletasks()
                work_w = self.root.winfo_screenwidth()
                work_h = self.root.winfo_screenheight()
            except Exception:
                work_w, work_h = 1100, 900

        # 1.2 新增（功能2）：保存工作区尺寸
        self.work_w = work_w
        self.work_h = work_h

        # 窗口尺寸：上限 1100x900，且不超过工作区（留边距），下限兜底防负数
        win_w = min(1100, max(320, work_w - 40))
        win_h = min(900, max(240, work_h - 60))
        win_x = int((work_w - win_w) / 2)
        win_y = int((work_h - win_h) / 2)
        self.root.geometry(f"{win_w}x{win_h}+{win_x}+{win_y}")

        # 最小尺寸：小屏上按工作区比例收缩，避免 minsize 大于屏幕
        min_w = min(900, max(680, int(work_w * 0.6)))
        min_h = min(700, max(520, int(work_h * 0.6)))
        self.root.minsize(min_w, min_h)

    # ==========================================================
    # 样式
    # ==========================================================
    def _apply_dark_theme(self):
        style = ttk.Style()

        # 关键：先切到 default 再切回 clam，强制清掉旧主题残留的样式
        # try:
        #     style.theme_use('default')
        # except Exception:
            # pass
        try:
            style.theme_use('clam')
        except Exception:
            pass


        style.configure('.',
            background=THEME['bg'],
            foreground=THEME['text'],
            fieldbackground=THEME['panel_2'],
            bordercolor=THEME['border'],
            lightcolor=THEME['panel'],
            darkcolor=THEME['panel'],
            troughcolor=THEME['panel_2'],
            focuscolor=THEME['accent'],
            font=FONT_UI,
        )

        style.configure('TFrame', background=THEME['bg'])
        style.configure('Panel.TFrame', background=THEME['panel'])
        style.configure('Card.TFrame', background=THEME['panel'])

        style.configure('TLabelframe',
            background=THEME['panel'],
            foreground=THEME['text_dim'],
            bordercolor=THEME['border'],
            relief='solid',
            borderwidth=1,
        )
        style.configure('TLabelframe.Label',
            background=THEME['panel'],
            foreground=THEME['accent'],
            font=FONT_TITLE,
        )

        style.configure('TLabel',
            background=THEME['panel'],
            foreground=THEME['text'],
            font=FONT_LABEL,
        )
        style.configure('Dim.TLabel',
            background=THEME['panel'],
            foreground=THEME['text_dim'],
            font=FONT_LABEL,
        )
        style.configure('Bg.TLabel',
            background=THEME['bg'],
            foreground=THEME['text'],
        )
        style.configure('Small.TLabel',
            background=THEME['panel'],
            foreground=THEME['text_dim'],
            font=FONT_SMALL,
        )

        style.configure('TButton',
            background=THEME['panel_2'],
            foreground=THEME['text'],
            bordercolor=THEME['border'],
            lightcolor=THEME['panel_2'],
            darkcolor=THEME['panel_2'],
            focuscolor=THEME['accent'],
            relief='flat',
            padding=(8, 5),
            font=FONT_UI,
        )
        style.map('TButton',
            background=[('active', THEME['accent']), ('pressed', THEME['accent'])],
            foreground=[('active', '#ffffff'), ('pressed', '#ffffff')],
            bordercolor=[('active', THEME['accent'])],
        )

        # 1.2 修改（布局紧凑化）：小尺寸按钮样式（中/EN、Help、Theme），
        # 收窄内边距避免 Connection 行0 在 340px 内溢出；其余属性继承 TButton
        style.configure('Small.TButton', padding=(4, 4))

        # 2026-11-03 用户要求：主题按钮恢复 🌙 颜文字。
        # 之前变豆腐块是因为普通 YaHei 字体渲染不出该字符，
        # 专用样式强制用 Segoe UI Emoji 字体（Windows 自带），保证显示为月亮；
        # 字号 9 与 Small.TButton 一致，保证与中/EN 按钮同高同宽
        style.configure('Emoji.TButton', padding=(4, 4),
                        font=('Segoe UI Emoji', 9))

        style.configure('Accent.TButton',
            background=THEME['accent'],
            foreground='#ffffff',
            bordercolor=THEME['accent'],
            lightcolor=THEME['accent'],
            darkcolor=THEME['accent'],
            font=('Microsoft YaHei UI', 9, 'bold'),
            padding=(8, 5),
        )
        style.map('Accent.TButton',
            background=[('active', THEME['accent_2']), ('pressed', THEME['accent_2'])],
            foreground=[('active', '#ffffff'), ('pressed', '#ffffff')],
        )

        style.configure('TEntry',
            fieldbackground=THEME['panel_2'],
            foreground=THEME['text'],
            bordercolor=THEME['border'],
            lightcolor=THEME['border'],
            darkcolor=THEME['border'],
            insertcolor=THEME['accent'],
            padding=4,
            font=FONT_UI,
        )
        style.map('TEntry',
            bordercolor=[('focus', THEME['accent'])],
            lightcolor=[('focus', THEME['accent'])],
            darkcolor=[('focus', THEME['accent'])],
        )

        style.configure('TCombobox',
            fieldbackground=THEME['panel_2'],
            background=THEME['panel_2'],
            foreground=THEME['text'],
            bordercolor=THEME['border'],
            arrowcolor=THEME['text_dim'],
            lightcolor=THEME['panel_2'],
            darkcolor=THEME['panel_2'],
            padding=3,
            font=FONT_UI,
        )
        style.map('TCombobox',
            fieldbackground=[('readonly', THEME['panel_2'])],
            foreground=[('readonly', THEME['text'])],
            bordercolor=[('focus', THEME['accent'])],
            arrowcolor=[('active', THEME['accent'])],
            selectbackground=[('readonly', THEME['panel_2'])],
            selectforeground=[('readonly', THEME['text'])],
        )
        self.root.option_add('*TCombobox*Listbox.background', THEME['panel_2'])
        self.root.option_add('*TCombobox*Listbox.foreground', THEME['text'])
        self.root.option_add('*TCombobox*Listbox.selectBackground', THEME['accent'])
        self.root.option_add('*TCombobox*Listbox.selectForeground', '#ffffff')
        self.root.option_add('*TCombobox*Listbox.font', FONT_UI)
        self.root.option_add('*TCombobox*Listbox.borderWidth', 0)
        self.root.option_add('*TCombobox*Listbox.relief', 'flat')
        self.root.option_add('*TCombobox*Listbox.highlightThickness', 0)

        style.configure('TCheckbutton',
            background=THEME['panel'],
            foreground=THEME['text'],
            focuscolor=THEME['accent'],
            font=FONT_UI,
        )
        style.map('TCheckbutton',
            background=[('active', THEME['panel'])],
            foreground=[('active', THEME['accent'])],
        )

        style.configure('TRadiobutton',
            background=THEME['panel'],
            foreground=THEME['text'],
            focuscolor=THEME['accent'],
            font=FONT_UI,
        )
        style.map('TRadiobutton',
            background=[('active', THEME['panel'])],
            foreground=[('active', THEME['accent'])],
        )

        style.configure('TScrollbar',
            background=THEME['panel_2'],
            troughcolor=THEME['bg'],
            bordercolor=THEME['border'],
            arrowcolor=THEME['text_dim'],
            lightcolor=THEME['panel_2'],
            darkcolor=THEME['panel_2'],
        )
        style.map('TScrollbar',
            background=[('active', THEME['accent'])],
            arrowcolor=[('active', '#ffffff')],
        )

        style.configure('TSeparator',
            background=THEME['border'],
        )

    def _init_channels(self, count):
        self.channel_count = count
        self.channels = []
        self.channel_values = []
        self.channel_diff_values = []
        self.channel_visible = []

        for i in range(count):
            start_idx = 2 + i * 2
            indices = [start_idx, start_idx + 1]
            self.channels.append({
                'indices': indices,
                'sign_bit': -1,
                'diff': False
            })
            self.channel_values.append(deque(maxlen=self.max_points))
            self.channel_diff_values.append(deque(maxlen=self.max_points))
            self.channel_visible.append(True)

    # ==========================================================
    # 1.2 新增（功能4）：i18n 辅助
    # ==========================================================
    def tr(self, key):
        """取当前语言的文案；缺失时回退英文，再缺失返回 key 本身"""
        lang = self.language if self.language in TEXTS else 'en'
        return TEXTS[lang].get(key, TEXTS['en'].get(key, key))

    def _reg(self, widget, key, attr='text'):
        """登记需要跟随语言切换的控件（功能4）"""
        self._i18n_widgets.append((widget, key, attr))
        return widget

    def toggle_language(self):
        """中/EN 切换：zh <-> en，立即刷新界面并写回配置"""
        self.language = 'en' if self.language == 'zh' else 'zh'
        self._apply_language()
        self._save_ui_config()

    def _group_title(self, key):
        """1.2 调整1：分组标题 = 语言文案 + 折叠箭头（箭头并入标题文字，
        展开形如 '  Connection  ▼  '，折叠形如 '  Connection  ▶  '）"""
        arrow = '▶' if self.collapsed_state.get(key, False) else '▼'
        return f"{self.tr(key)}{arrow}  "

    def _apply_language(self):
        """遍历登记过的控件，按当前语言重设文案。
        注意（冒烟探针实测）：setattr(widget, attr, v) 对 Tkinter 控件是静默
        无效的（只写 Python 实例属性，不触发 Tcl configure），必须用
        widget.configure(**{attr: v})。
        1.2 调整1：三个可折叠组的标题按当前折叠状态拼接箭头。
        动态文本（Connect/Disconnect 按钮）按连接状态取词。"""
        for widget, key, attr in list(self._i18n_widgets):
            try:
                if key in ('connection', 'datalog', 'framecfg'):
                    widget.configure(**{attr: self._group_title(key)})
                else:
                    widget.configure(**{attr: self.tr(key)})
            except Exception:
                pass
        try:
            self.connect_btn.config(
                text=self.tr('disconnect' if self.is_connected else 'connect'))
        except Exception:
            pass

    # ==========================================================
    # 1.2 新增（功能1）：分组折叠
    # 1.2 重构：body 容器包裹方案——每个可折叠组在 labelframe 内放一个
    # body(tk.Frame)，组内全部控件都是 body 的子控件；折叠/展开只操作
    # body 这一个控件的 grid_remove/grid，组内控件永远不动。
    # （替代旧的"收集子控件 grid_info 快照 + 逐个恢复"方案：
    #   该方案在 serial_frame/udp_frame 同格叠放、控件晚建等场景下时序错乱，
    #   曾导致整组消失 / 折叠后残留空壳两个 bug。）
    # ==========================================================
    def _make_collapsible(self, labelframe, key):
        """把一个 ttk.LabelFrame 变成可折叠分组（body 容器方案）。
        必须在创建组内子控件【之前】调用（子控件 parent 要用返回的 body）：
        - 在 labelframe 内创建 body = tk.Frame，grid(row=0, column=0) 占满
        - 返回 body，调用方把组内所有控件的 parent 设为该 body
        - 折叠 = body.grid_remove()；展开 = body.grid()
        - 箭头并入标题文字（_group_title）；点击标题行（y<=16）切换折叠
        - 按当前 collapsed_state（默认/小屏/json 恢复值）立即应用初始状态"""
        body = tk.Frame(labelframe, bg=THEME['panel'])
        body.grid(row=0, column=0, sticky=(tk.W, tk.E, tk.N, tk.S))
        labelframe.columnconfigure(0, weight=1)
        # 点击标题行切换折叠（ttk.Labelframe 标题为内部元素，事件送达 labelframe）
        labelframe.bind('<Button-1>',
                        lambda e, k=key, lf=labelframe: self._on_group_title_click(e, k, lf))
        self._collapsible[key] = {'frame': labelframe, 'body': body}
        # 按初始状态应用（默认展开 / 小屏或配置文件要求折叠）
        self._set_group_collapsed(key, self.collapsed_state.get(key, False))
        return body

    def _on_group_title_click(self, event, key, labelframe):
        """标题行点击 → 切换折叠。仅标题行高度内、且事件确来自
        labelframe 本身才响应（防御性保护：组内控件点击不会冒泡到此处）。"""
        try:
            if event.widget is not labelframe:
                return
            if event.y <= 16:
                self._toggle_group(key)
        except Exception:
            pass

    def _toggle_group(self, key):
        self._set_group_collapsed(key, not self.collapsed_state.get(key, False))
        # 功能3：每次切换立即写回
        self._save_ui_config()

    def _title_row_height(self):
        """折叠态的标题行高度：按标题字体（TLabelframe.Label，10 号粗体）行高估算，
        失败兜底 30px。随 DPI 缩放自适应。"""
        try:
            import tkinter.font as tkfont
            f = tkfont.Font(family='Microsoft YaHei UI', size=10, weight='bold')
            return f.metrics('linespace') + 12
        except Exception:
            return 30

    def _set_group_collapsed(self, key, collapsed):
        item = self._collapsible.get(key)
        if not item:
            return
        self.collapsed_state[key] = bool(collapsed)
        frame = item['frame']
        body = item['body']
        if collapsed:
            # 1.2 重构：只操作 body 一个控件，组内控件永远不动
            try:
                body.grid_remove()
            except Exception:
                pass
            try:
                # 探针实测（Tk 8.6.15）：容器请求高度只增不减，子控件移除后
                # labelframe 请求高度仍停留在旧值——这是"折叠后残留空壳"的根源。
                # 因此折叠时冻结 propagate 并把高度固定为标题行高度。
                frame.grid_propagate(False)
                frame.configure(height=self._title_row_height())
            except Exception:
                pass
        else:
            try:
                body.grid()
            except Exception:
                pass
            try:
                # 展开：恢复 propagate，高度由 body 内控件重新撑起（往返可靠，探针已验证）
                frame.grid_propagate(True)
            except Exception:
                pass
        # 箭头并入标题文字，折叠状态切换时同步更新 text
        try:
            frame.configure(text=self._group_title(key))
        except Exception:
            pass

    # ==========================================================
    # 1.2 新增（功能3）：ui_config.json 持久化
    # 路径与图标加载相同的 base 目录逻辑（Nuitka 下 __nuitka_binary_dir__，
    # 否则脚本目录），文件名 ui_config.json
    # ==========================================================
    def _get_config_path(self):
        """界面配置文件位置（2026-11-03 修正）。
        原实现优先用 __nuitka_binary_dir__，那是 onefile 的【临时解包目录】，
        进程一退出就被清掉——等于语言/折叠状态没记忆，而且文件藏在你找不到的地方。
        改为：打包运行 → exe 同目录（可见、可手动删）；源码运行 → 脚本目录；
        都不可写时回退用户主目录。"""
        candidates = []
        try:
            if getattr(sys, 'frozen', False) or getattr(sys, '__nuitka_binary_dir__', None):
                candidates.append(os.path.dirname(os.path.abspath(sys.executable)))
        except Exception:
            pass
        try:
            candidates.append(os.path.dirname(os.path.abspath(__file__)))
        except Exception:
            pass
        try:
            candidates.append(os.path.expanduser('~'))
        except Exception:
            pass
        for base in candidates:
            try:
                if base and os.path.isdir(base) and os.access(base, os.W_OK):
                    return os.path.join(base, "ui_config.json")
            except Exception:
                pass
        try:
            return os.path.join(os.getcwd(), "ui_config.json")
        except Exception:
            return "ui_config.json"

    def _load_ui_config(self):
        """启动时读取 ui_config.json（只恢复语言）。
        2026-11-03 用户要求：折叠状态不再跨启动恢复，启动一律全展开；
        文件中的 collapsed 字段继续写入但不读回，避免旧文件把界面折叠起来。"""
        try:
            with open(self._config_path, 'r', encoding='utf-8') as f:
                data = json.load(f)
        except Exception:
            return
        if not isinstance(data, dict):
            return
        lang = data.get('language')
        if lang in ('zh', 'en'):
            self.language = lang

    def _save_ui_config(self):
        """立即写回 ui_config.json（折叠切换/语言切换时调用，失败静默）"""
        try:
            data = {
                'collapsed': {
                    'connection': bool(self.collapsed_state.get('connection', False)),
                    'datalog':    bool(self.collapsed_state.get('datalog', False)),
                    'framecfg':   bool(self.collapsed_state.get('framecfg', False)),
                },
                'language': self.language,
            }
            with open(self._config_path, 'w', encoding='utf-8') as f:
                json.dump(data, f, ensure_ascii=False, indent=2)
        except Exception:
            pass

    def _on_close(self):
        """退出前再保存一次配置，然后关闭窗口"""
        self._save_ui_config()
        try:
            self.root.destroy()
        except Exception:
            pass

    def create_widgets(self):
        main_frame = tk.Frame(self.root, bg=THEME['panel'], padx=5, pady=5)
        main_frame.grid(row=0, column=0, sticky=(tk.W, tk.E, tk.N, tk.S))
        self.root.columnconfigure(0, weight=1)
        self.root.rowconfigure(0, weight=1)

        main_frame.columnconfigure(0, weight=0)
        main_frame.columnconfigure(1, weight=1)
        main_frame.rowconfigure(0, weight=1)

        # ========== 左侧设置区 ==========
        left_frame = tk.Frame(main_frame, width=364, bg=THEME['panel'])
        left_frame.grid(row=0, column=0, sticky=(tk.W, tk.E, tk.N, tk.S), padx=(0, 5))
        left_frame.grid_propagate(False)
        left_frame.columnconfigure(0, weight=1)

        # ---------- Connection ----------
        port_frame = ttk.LabelFrame(left_frame, text=self.tr('connection'), padding="8")
        port_frame.grid(row=0, column=0, sticky=(tk.W, tk.E), pady=3)
        self._reg(port_frame, 'connection')
        # 1.2 重构：body 容器包裹——先建 body，组内所有控件 parent 均为 body；
        # 原 port_frame 上的列配置移到 body 上。
        # row=0 布局（调整2）：col0 标签 + col1 下拉框紧凑靠左（不拉伸），
        # 中/EN 按钮紧跟下拉框（col2）；col3 拉伸把 Help 与 🌙（col4）用 sticky=E 推到行尾
        port_body = self._make_collapsible(port_frame, 'connection')
        port_body.columnconfigure(0, weight=1)

        # 2026-11-03 布局修复（用户反馈 Help/中EN/Theme 消失）：
        # row=0 改用 pack 容器——左侧 Interface 标签+下拉框 side=LEFT 靠左，
        # 右侧 Help/中EN/Theme 用 side=RIGHT 靠右。pack 不与其他行共用网格列宽，
        # 从机制上杜绝"被 Interface/Port 行撑列后挤出可视区"。
        top_row = tk.Frame(port_body, bg=THEME['panel'])
        top_row.grid(row=0, column=0, columnspan=4, padx=2, pady=2, sticky=(tk.W, tk.E))
        # 右侧两个按钮先占位（side=RIGHT 保证优先分配空间）
        # 2026-11-03 用户反馈：中/EN+Help+Theme 三个挤在一行太窄，Help 已挪到
        # 右侧 Data Plot 标题行行尾（见下方 switch_frame），首行只留 中/EN + Theme
        # 2026-11-03 用户要求：Theme 改回 🌙 颜文字（Emoji.TButton 用 Segoe UI Emoji
        # 字体渲染，避免 100% DPI 下变豆腐块；文字固定，不参与 i18n）
        self.theme_btn = ttk.Button(top_row, text="🌙", width=3,
                                    command=self.toggle_theme,
                                    style='Emoji.TButton')
        self.theme_btn.pack(side=tk.RIGHT, padx=(3, 0))
        self.lang_btn = self._reg(ttk.Button(top_row, text=self.tr('lang_btn'),
                                             command=self.toggle_language, width=3,
                                             style='Small.TButton'),
                                  'lang_btn')
        self.lang_btn.pack(side=tk.RIGHT)
        # 左侧 Interface 标签 + 下拉框
        self._reg(ttk.Label(top_row, text=self.tr('interface')),
                  'interface').pack(side=tk.LEFT, padx=(0, 3))
        self.interface_var = tk.StringVar(value="Serial")
        interface_combo = ttk.Combobox(top_row, textvariable=self.interface_var,
                                    values=["Serial", "UDP"], width=6, state="readonly")
        interface_combo.pack(side=tk.LEFT)
        interface_combo.bind("<<ComboboxSelected>>", self._on_interface_change)

        ttk.Separator(port_body, orient=tk.HORIZONTAL).grid(row=1, column=0, columnspan=4, padx=3, pady=5, sticky=(tk.W, tk.E))

        # 1.2 重构：serial_frame/udp_frame 的 parent 改为 body；
        # 二者叠在 body 的 row=2 同一格里，_on_interface_change 的显隐逻辑不变
        self.serial_frame = tk.Frame(port_body, bg=THEME['panel'])
        self.serial_frame.grid(row=2, column=0, columnspan=3, sticky=(tk.W, tk.E))
        self.serial_frame.columnconfigure(1, weight=1)

        self._reg(ttk.Label(self.serial_frame, text=self.tr('port')),
                  'port').grid(row=0, column=0, padx=2, pady=2, sticky=tk.W)
        self.port_var = tk.StringVar()
        self.port_combo = ttk.Combobox(self.serial_frame, textvariable=self.port_var, width=12)
        self.port_combo.grid(row=0, column=1, padx=2, pady=2, sticky=(tk.W, tk.E))
        # 2026-11-03 用户要求：Deep Scan 去掉，只保留一个 Refresh。
        # Refresh 默认做 pyserial 枚举 + 注册表读取（已覆盖绝大多数场景），
        # 腾出的宽度全部给端口下拉框，避免下拉框被挤成一条缝。
        self._reg(ttk.Button(self.serial_frame, text=self.tr('refresh'),
                             command=self.refresh_ports, width=7),
                  'refresh').grid(row=0, column=2, padx=2, pady=2)

        self._reg(ttk.Label(self.serial_frame, text=self.tr('baud')),
                  'baud').grid(row=1, column=0, padx=2, pady=2, sticky=tk.W)
        self.baud_var = tk.StringVar(value="1000000")
        baud_combo = ttk.Combobox(self.serial_frame, textvariable=self.baud_var,
                    values=["9600", "19200", "38400", "57600", "115200", "230400", "460800", "921600",
                            "1000000", "1500000", "2000000", "2500000", "3000000"],
                    width=12)
        baud_combo.grid(row=1, column=1, columnspan=2, padx=3, pady=2, sticky=(tk.W, tk.E))

        self.udp_frame = tk.Frame(port_body, bg=THEME['panel'])
        self.udp_frame.grid(row=2, column=0, columnspan=3, sticky=(tk.W, tk.E))
        self.udp_frame.columnconfigure(1, weight=1)

        self._reg(ttk.Label(self.udp_frame, text=self.tr('local_ip')),
                  'local_ip').grid(row=0, column=0, padx=3, pady=2, sticky=tk.W)
        self.udp_local_ip_var = tk.StringVar(value="0.0.0.0")
        ttk.Entry(self.udp_frame, textvariable=self.udp_local_ip_var, width=12).grid(row=0, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        self._reg(ttk.Label(self.udp_frame, text=self.tr('local_port')),
                  'local_port').grid(row=1, column=0, padx=3, pady=2, sticky=tk.W)
        self.udp_local_port_var = tk.StringVar(value="8080")
        ttk.Entry(self.udp_frame, textvariable=self.udp_local_port_var, width=12).grid(row=1, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        self._reg(ttk.Label(self.udp_frame, text=self.tr('remote_ip')),
                  'remote_ip').grid(row=2, column=0, padx=3, pady=2, sticky=tk.W)
        self.udp_remote_ip_var = tk.StringVar(value="")
        ttk.Entry(self.udp_frame, textvariable=self.udp_remote_ip_var, width=12).grid(row=2, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        self._reg(ttk.Label(self.udp_frame, text=self.tr('remote_port')),
                  'remote_port').grid(row=3, column=0, padx=3, pady=2, sticky=tk.W)
        self.udp_remote_port_var = tk.StringVar(value="")
        ttk.Entry(self.udp_frame, textvariable=self.udp_remote_port_var, width=12).grid(row=3, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        self.connect_btn = self._reg(ttk.Button(port_body, text=self.tr('connect'),
                                    command=self.toggle_connection,
                                    style='Accent.TButton'), 'connect')
        self.connect_btn.grid(row=3, column=0, columnspan=3, padx=3, pady=5, sticky=(tk.W, tk.E))

        # 2026-11-03 用户要求：Load File / Analyze Clipboard 两个按钮从 Connection 组
        # 移到 Frame Configuration 组底部那一行（见下方 frame_body row=3），
        # Connection 组到 Connect 结束，组尾分隔线随之删除。
        # 1.2 重构：Connection 组折叠已在上面 _make_collapsible 建好 body 时生效，
        # 组尾不再需要二次调用（旧的快照收集调用已删除）

        # ---------- Data Log ----------
        log_frame = ttk.LabelFrame(left_frame, text=self.tr('datalog'), padding="8")
        log_frame.grid(row=1, column=0, sticky=(tk.W, tk.E), pady=3)
        self._reg(log_frame, 'datalog')
        # 1.2 重构：body 容器包裹，原 log_frame 的列配置移到 body 上
        # （col0/col1/col3 自然宽度，col2 文件名标签拉伸）
        log_body = self._make_collapsible(log_frame, 'datalog')
        log_body.columnconfigure(2, weight=1)

        self.extracted_save_var = tk.BooleanVar(value=False)
        self._reg(ttk.Checkbutton(log_body, text=self.tr('save_extracted'),
                                  variable=self.extracted_save_var,
                                  command=self.toggle_extracted_save),
                  'save_extracted').grid(row=0, column=0, padx=2, pady=2, sticky=tk.W)
        self._reg(ttk.Button(log_body, text=self.tr('csv_file'),
                             command=self.select_extracted_save_file, width=10),
                  'csv_file').grid(row=0, column=1, padx=2, pady=2, sticky=tk.W)
        self.extracted_save_label = ttk.Label(log_body, text="(no file)", style='Small.TLabel',
                                                wraplength=96)
        self.extracted_save_label.grid(row=0, column=2, columnspan=2, padx=2, pady=2, sticky=tk.W)

        self.auto_save_var = tk.BooleanVar(value=False)
        self._reg(ttk.Checkbutton(log_body, text=self.tr('auto_save'),
                                  variable=self.auto_save_var,
                                  command=self.toggle_auto_save),
                  'auto_save').grid(row=1, column=0, padx=2, pady=2, sticky=tk.W)
        self._reg(ttk.Button(log_body, text=self.tr('save_file'),
                             command=self.select_save_file, width=10),
                  'save_file').grid(row=1, column=1, padx=2, pady=2, sticky=tk.W)
        self.save_file_label = ttk.Label(log_body, text="(no file)", style='Small.TLabel',
                                        wraplength=96)
        self.save_file_label.grid(row=1, column=2, columnspan=2, padx=2, pady=2, sticky=tk.W)
        self._attach_path_tooltip(self.save_file_label, lambda: getattr(self, 'auto_save_file', ''))
        self._attach_path_tooltip(self.extracted_save_label, lambda: getattr(self, 'extracted_save_file', ''))
        # 1.2 重构：Data Log 组折叠已随 body 生效，组尾不再调用 _make_collapsible

        # ---------- Frame Configuration ----------
        frame_cfg = ttk.LabelFrame(left_frame, text=self.tr('framecfg'), padding="8")
        frame_cfg.grid(row=2, column=0, sticky=(tk.W, tk.E), pady=3)
        self._reg(frame_cfg, 'framecfg')
        # 1.2 重构：body 容器包裹，原 frame_cfg 的列配置移到 body 上
        frame_body = self._make_collapsible(frame_cfg, 'framecfg')
        frame_body.columnconfigure(1, weight=1)
        frame_body.columnconfigure(3, weight=1)

        self._reg(ttk.Label(frame_body, text=self.tr('header')),
                  'header').grid(row=0, column=0, padx=3, pady=2, sticky=tk.W)
        self.header_var = tk.StringVar(value="EB 90")
        ttk.Entry(frame_body, textvariable=self.header_var, width=8).grid(row=0, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        self._reg(ttk.Label(frame_body, text=self.tr('length')),
                  'length').grid(row=0, column=2, padx=3, pady=2, sticky=tk.W)
        self.length_var = tk.StringVar(value="11")
        ttk.Entry(frame_body, textvariable=self.length_var, width=6).grid(row=0, column=3, padx=3, pady=2, sticky=(tk.W, tk.E))

        self._reg(ttk.Label(frame_body, text=self.tr('byte_order')),
                  'byte_order').grid(row=1, column=0, padx=3, pady=2, sticky=tk.W)
        self.order_var = tk.StringVar(value="big")
        ttk.Combobox(frame_body, textvariable=self.order_var, values=["big", "little"],
                     width=8, state="readonly").grid(row=1, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        self._reg(ttk.Label(frame_body, text=self.tr('samples_pkt')),
                  'samples_pkt').grid(row=1, column=2, padx=3, pady=2, sticky=tk.W)
        self.samples_var = tk.StringVar(value="1")
        ttk.Entry(frame_body, textvariable=self.samples_var, width=6).grid(row=1, column=3, padx=3, pady=2, sticky=(tk.W, tk.E))

        self._reg(ttk.Label(frame_body, text=self.tr('max_points')),
                  'max_points').grid(row=2, column=0, padx=3, pady=2, sticky=tk.W)
        self.max_pts_var = tk.StringVar(value="500")
        ttk.Entry(frame_body, textvariable=self.max_pts_var, width=8).grid(row=2, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        # 2026-11-03 用户要求：Apply Frame 上移一行，紧跟在 Max Pts 输入框后面
        # （即 Smp 那一列的正下方），不再与 Clear Plot 并排占一整行
        self._reg(ttk.Button(frame_body, text=self.tr('apply_frame'),
                             command=self.apply_frame_config, style='Accent.TButton'),
                  'apply_frame').grid(row=2, column=2, columnspan=2, padx=(3, 0), pady=2,
                                      sticky=(tk.W, tk.E))

        # 2026-11-03 用户要求：Load File / Analyze Clipboard 移到这一行
        # （原来是 Apply Frame | Clear Plot 的位置；Clear Plot 已移到右侧 Send 标题行）
        btn_row = tk.Frame(frame_body, bg=THEME['panel'])
        btn_row.grid(row=3, column=0, columnspan=4, sticky=(tk.W, tk.E), pady=(5, 0))
        btn_row.columnconfigure(0, weight=1)
        btn_row.columnconfigure(1, weight=1)
        self._reg(ttk.Button(btn_row, text=self.tr('load_file'),
                             command=self.load_file),
                  'load_file').grid(row=0, column=0, padx=(0, 3), pady=0, sticky=(tk.W, tk.E))
        self._reg(ttk.Button(btn_row, text=self.tr('analyze_clip'),
                             command=self.analyze_clipboard),
                  'analyze_clip').grid(row=0, column=1, padx=(3, 0), pady=0, sticky=(tk.W, tk.E))
        # 1.2 重构：Frame Configuration 组折叠已随 body 生效，组尾不再调用 _make_collapsible

        # ---------- Channel Configuration ----------
        # 1.2 注意：通道配置是主工作区，不做折叠
        chan_frame = ttk.LabelFrame(left_frame, text=self.tr('chancfg'), padding="8")
        chan_frame.grid(row=3, column=0, sticky=(tk.W, tk.E, tk.N, tk.S), pady=3)
        self._reg(chan_frame, 'chancfg')
        chan_frame.columnconfigure(0, weight=1)
        chan_frame.rowconfigure(2, weight=1)
        left_frame.rowconfigure(3, weight=1)

        self.chan_top = tk.Frame(chan_frame, bg=THEME['panel'])
        self.chan_top.grid(row=0, column=0, sticky=(tk.W, tk.E), pady=(0, 3))
        chan_top = self.chan_top
        self._reg(ttk.Label(chan_top, text=self.tr('channels')),
                  'channels').pack(side=tk.LEFT, padx=(0, 5))
        self.channel_count_var = tk.StringVar(value="1")
        ttk.Entry(chan_top, textvariable=self.channel_count_var, width=8).pack(side=tk.LEFT, padx=(0, 5))
        self._reg(ttk.Button(chan_top, text=self.tr('apply'),
                             command=self.apply_channel_config),
                  'apply').pack(side=tk.LEFT)

        self.color_legend_frame = tk.Frame(chan_frame, bg=THEME['panel'])
        self.color_legend_frame.grid(row=1, column=0, sticky=(tk.W, tk.E), pady=3)

        scroll_wrap = tk.Frame(chan_frame, bg=THEME['panel'])
        scroll_wrap.grid(row=2, column=0, sticky=(tk.W, tk.E, tk.N, tk.S))
        scroll_wrap.columnconfigure(0, weight=1)
        scroll_wrap.rowconfigure(0, weight=1)

        self.channels_canvas = tk.Canvas(scroll_wrap, highlightthickness=0,
                                        bg=THEME['panel'], bd=0)
        self.channels_scroll = ttk.Scrollbar(scroll_wrap, orient="vertical",
                                        command=self.channels_canvas.yview)
        self.channels_container = tk.Frame(self.channels_canvas, bg=THEME['panel'])

        self.channels_container.bind(
            "<Configure>",
            lambda e: self.channels_canvas.configure(scrollregion=self.channels_canvas.bbox("all"))
        )
        self.channels_canvas.create_window((0, 0), window=self.channels_container, anchor="nw")
        self.channels_canvas.configure(yscrollcommand=self.channels_scroll.set)

        self.channels_canvas.grid(row=0, column=0, sticky=(tk.W, tk.E, tk.N, tk.S))
        self.channels_scroll.grid(row=0, column=1, sticky=(tk.N, tk.S))

        self._create_channel_widgets()
        self._update_color_legend()

        # ========== 右侧主区域 ==========
        right_frame = tk.Frame(main_frame, bg=THEME['panel'])
        right_frame.grid(row=0, column=1, sticky=(tk.W, tk.E, tk.N, tk.S))
        right_frame.columnconfigure(0, weight=1)
        right_frame.rowconfigure(0, weight=1)

        plot_frame = ttk.LabelFrame(right_frame, text=self.tr('plot_title'), padding="5")
        self._reg(plot_frame, 'plot_title')
        plot_frame.grid(row=0, column=0, sticky=(tk.W, tk.E, tk.N, tk.S))
        plot_frame.rowconfigure(1, weight=1)
        plot_frame.columnconfigure(0, weight=1)

        self.view_mode_var = tk.StringVar(value="绘图")
        # 2026-11-03：Data Plot 标题行改为左右两段——左侧 绘图/原始 切换，
        # 右侧 Help 按钮（从 Connection 首行挪来，那里三个按钮太挤）
        plot_header = tk.Frame(plot_frame, bg=THEME['panel'])
        plot_header.grid(row=0, column=0, sticky=(tk.W, tk.E), pady=(0, 3))
        self.switch_frame = tk.Frame(plot_header, bg=THEME['panel'])
        self.switch_frame.pack(side=tk.LEFT)
        # 2026-11-03：绘图/原始 切换按钮纳入 i18n——只改显示文字(text)，
        # 内部 value 固定为 "绘图"/"原始"，视图切换逻辑不受语言影响
        self._reg(self._make_switch_btn(self.switch_frame, self.tr('view_plot'), "绘图"),
                  'view_plot').pack(side=tk.LEFT, padx=2)
        self._reg(self._make_switch_btn(self.switch_frame, self.tr('view_raw'), "原始"),
                  'view_raw').pack(side=tk.LEFT, padx=2)
        self._reg(ttk.Button(plot_header, text=self.tr('help'),
                             command=self.show_help, width=5,
                             style='Small.TButton'),
                  'help').pack(side=tk.RIGHT)

        self.fig = Figure(figsize=(8, 5), dpi=100, facecolor=THEME['plot_bg'])
        self.ax = self.fig.add_subplot(111)
        self.ax.set_facecolor(THEME['plot_bg'])
        self.ax.set_xlabel('Frame Number', color=THEME['text_dim'])
        self.ax.set_ylabel('Value', color=THEME['text_dim'])
        self.ax.grid(True, color=THEME['plot_grid'], alpha=0.8, linewidth=0.6)
        self.ax.tick_params(colors=THEME['text_dim'], which='both')
        for spine in self.ax.spines.values():
            spine.set_color(THEME['border'])

        self.fig.subplots_adjust(left=0.08, right=0.99, top=0.99, bottom=0.08)

        self.canvas = FigureCanvasTkAgg(self.fig, master=plot_frame)
        self.canvas.draw()
        self.canvas.get_tk_widget().configure(bg=THEME['plot_bg'], highlightthickness=0)
        self.canvas.get_tk_widget().grid(row=1, column=0, sticky=(tk.W, tk.E, tk.N, tk.S))

        self.toolbar_frame = tk.Frame(plot_frame, bg=THEME['panel'])
        self.toolbar_frame.grid(row=2, column=0, sticky=(tk.W, tk.E))
        self.toolbar = NavigationToolbar2Tk(self.canvas, self.toolbar_frame)
        self.toolbar.update()

        # 2026-11-03 用户指定：清空绘图改为【图标按钮】（不要文字），位置在
        # matplotlib 工具栏图标组末尾（保存图标右侧的空位），不再放行尾——
        # 之前放行尾会挡住 (x, y) 坐标显示。扁平无边框，与工具栏图标同一套风格
        # （主题切换时由 _apply_toolbar_dark_style 自动刷新配色）。
        # 想换图标：改下面 text="🗑" 里的字符即可。
        # 2026-11-03 用户反馈：emoji 按钮尺寸随字体浮动，与左侧 matplotlib
        # 位图图标（24×24）不一致。用一张透明 24×24 PhotoImage 撑住按钮尺寸，
        # emoji 以 compound='center' 叠在正中——按钮大小与左侧图标完全一致，
        # 且透明图片不显示任何内容、随主题变色无影响。必须持有引用防 GC。
        self._trash_icon_img = tk.PhotoImage(width=24, height=24)
        self.clear_plot_tb_btn = tk.Button(self.toolbar, text="🗑",
                                           image=self._trash_icon_img,
                                           compound='center',
                                           command=self.clear_plot,
                                           bd=0, relief='flat',
                                           bg=THEME['panel'], fg=THEME['text'],
                                           activebackground=THEME['panel_2'],
                                           activeforeground=THEME['accent'],
                                           font=('Segoe UI Emoji', 14),
                                           highlightthickness=0, padx=0, pady=0)
        try:
            self.clear_plot_tb_btn.pack(side=tk.LEFT, padx=2)
        except Exception:
            # 兜底：若 matplotlib 内部改用 grid 布局导致 pack 失败，则叠加放置
            self.clear_plot_tb_btn.place(x=170, y=2)
        # 2026-11-03 用户反馈：图标按钮没有提示不知道是干嘛的，悬停显示功能名（随语言）
        self._attach_tooltip(self.clear_plot_tb_btn, lambda: self.tr('clear_plot'))

        self._apply_toolbar_dark_style()

        self.root.after(200, self._patch_home_button)

        self.canvas.mpl_connect('scroll_event', self._on_mouse_wheel)

        raw_container = tk.Frame(plot_frame, bg=THEME['panel'])
        raw_container.grid(row=1, column=0, sticky=(tk.W, tk.E, tk.N, tk.S))
        raw_container.columnconfigure(0, weight=1)
        raw_container.rowconfigure(0, weight=1)
        self.raw_text = tk.Text(raw_container, state=tk.DISABLED, font=FONT_MONO, wrap=tk.NONE,
                                bg=THEME['plot_bg'], fg=THEME['text'],
                                insertbackground=THEME['accent'],
                                selectbackground=THEME['accent'],
                                selectforeground='#ffffff',
                                relief='flat', bd=0, highlightthickness=1,
                                highlightbackground=THEME['border'],
                                highlightcolor=THEME['accent'])
        self.raw_scroll = ttk.Scrollbar(raw_container, orient=tk.VERTICAL, command=self.raw_text.yview)
        self.raw_text.configure(yscrollcommand=self.raw_scroll.set)
        self.raw_text.grid(row=0, column=0, sticky=(tk.W, tk.E, tk.N, tk.S))
        self.raw_scroll.grid(row=0, column=1, sticky=(tk.N, tk.S))
        self.raw_container = raw_container
        self.raw_container.grid_remove()

        self._init_plot_lines()

        # --- Send ---
        # 2026-11-03 用户要求：红线处空白挤掉还给绘图区。
        # LabelFrame 即使 text="" 仍会保留标题带高度，因此改用普通 tk.Frame +
        # 1px 边框（外观与其他分组框一致），上下零预留，Data 行紧贴工具栏行。
        tx_frame = tk.Frame(right_frame, bg=THEME['panel'],
                            highlightthickness=1,
                            highlightbackground=THEME['border'])
        tx_frame.grid(row=1, column=0, sticky=(tk.W, tk.E), pady=(2, 0))
        tx_frame.columnconfigure(1, weight=1)

        ttk.Label(tx_frame, text="Data:").grid(row=0, column=0, padx=3, pady=2, sticky=tk.W)
        self.tx_data_var = tk.StringVar(value="EB 90 01 02 03 04 05 06 07 08 00")
        self.tx_entry = ttk.Entry(tx_frame, textvariable=self.tx_data_var)
        self.tx_entry.grid(row=0, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        self.tx_format_var = tk.StringVar(value="hex")
        self._make_switch_btn_tx(tx_frame, "HEX", "hex").grid(row=0, column=2, padx=2, pady=2)
        self._make_switch_btn_tx(tx_frame, "ASCII", "ascii").grid(row=0, column=3, padx=2, pady=2)

        # 2026-11-03：Send/History 纳入 i18n（中：发送 / 历史发送）。
        # 【按钮宽度在这里调】改 width= 数字即可（单位≈一个英文字符宽）：
        self._reg(ttk.Button(tx_frame, text=self.tr('send'), command=self.send_data,
                style='Accent.TButton', width=7),
                  'send').grid(row=0, column=4, padx=5, pady=2)
        self._reg(ttk.Button(tx_frame, text=self.tr('history'),
                             command=self.show_send_history, width=8),
                  'history').grid(row=0, column=5, padx=3, pady=2)

        self.tx_count = 0
        self.send_history = []
        self.max_history = 50
        self.tx_count_var = tk.StringVar(value="TX: 0")
        # 2026-11-03 用户要求：TX 计数从本行移到最下面状态栏行尾

        info_frame = tk.Frame(right_frame, bg=THEME['bg'])
        info_frame.grid(row=2, column=0, sticky=(tk.W, tk.E), pady=(5, 0))

        # 状态栏行：左侧状态文本（原样），行尾追加 TX 计数
        status_row = tk.Frame(info_frame, bg=THEME['panel'],
                              highlightthickness=1,
                              highlightbackground=THEME['border'])
        status_row.pack(fill=tk.X)
        self.status_var = tk.StringVar(value="Disconnected | Frames: 0 | Channels: 1")
        self.status_label = tk.Label(status_row, textvariable=self.status_var,
                                    bg=THEME['panel'], fg=THEME['text_dim'],
                                    font=FONT_UI, anchor=tk.W, padx=10, pady=5)
        self.status_label.pack(side=tk.LEFT, fill=tk.X, expand=True)
        self.tx_count_label = tk.Label(status_row, textvariable=self.tx_count_var,
                                       bg=THEME['panel'], fg=THEME['text_dim'],
                                       font=FONT_UI, padx=10)
        self.tx_count_label.pack(side=tk.RIGHT)

    # ==========================================================
    # 主题切换
    # ==========================================================
    def toggle_theme(self):
        """循环切换：深色 → 原始 → 淡绿 → 淡蓝 → 米黄 → 深色"""
        global THEME

        self.theme_index = (self.theme_index + 1) % len(THEME_LIST)
        name, theme_dict, icon = THEME_LIST[self.theme_index]

        THEME.clear()
        THEME.update(theme_dict)

        # 1) 重建 ttk 样式
        self._apply_dark_theme()

        # 2) 主窗口背景
        self.root.configure(bg=THEME['bg'])

        # 3) 标题栏
        try:
            apply_dark_titlebar(self.root, THEME['titlebar'])
        except Exception:
            pass

        # 4) matplotlib 图区
        try:
            self.fig.set_facecolor(THEME['plot_bg'])
            self.ax.set_facecolor(THEME['plot_bg'])
            self.ax.xaxis.label.set_color(THEME['text_dim'])
            self.ax.yaxis.label.set_color(THEME['text_dim'])
            self.ax.tick_params(colors=THEME['text_dim'], which='both')
            for spine in self.ax.spines.values():
                spine.set_color(THEME['border'])
            self.ax.grid(True, color=THEME['plot_grid'], alpha=0.8, linewidth=0.6)
            leg = self.ax.get_legend()
            if leg:
                leg.get_frame().set_facecolor(THEME['panel'])
                leg.get_frame().set_edgecolor(THEME['border'])
                for text in leg.get_texts():
                    text.set_color(THEME['text'])
            self.canvas.get_tk_widget().configure(bg=THEME['plot_bg'])
            self.canvas.draw()
        except Exception:
            pass

        # 5) 遍历所有 tk 控件刷新颜色
        self._refresh_widgets_colors(self.root)

        # 5.1) 单独刷新通道画布（避免黑色块）
        try:
            self.channels_canvas.configure(bg=THEME['panel'])
            self.channels_container.configure(bg=THEME['panel'])
        except Exception:
            pass

        # 5.2) 单独刷新视图切换按钮（绘图/原始）
        try:
            for child in self.switch_frame.winfo_children():
                child.configure(
                    bg=THEME['panel_2'], fg=THEME['text'],
                    selectcolor=THEME['accent'],
                    activebackground=THEME['accent'],
                    activeforeground='#ffffff',
                )
        except Exception:
            pass

        # 6) 重设 toolbar 深色/浅色
        try:
            self._apply_toolbar_dark_style()
        except Exception:
            pass

        # 7) 主题按钮已改为文字按钮（Theme/主题，i18n 管理），
        #    不再把主题图标写回按钮文本

        # 8) 重建通道行（里面有很多硬编码颜色）
        try:
            self._create_channel_widgets()
        except Exception:
            pass

        # 8.1) 重建颜色图例（切换主题后色块需要重画）
        try:
            self._update_color_legend()
        except Exception:
            pass

        # 9) 更新状态栏颜色
        try:
            self.status_label.configure(bg=THEME['panel'], fg=THEME['text_dim'],
                                        highlightbackground=THEME['border'])
        except Exception:
            pass
        # 10) 手动刷新所有卡片内的 Frame（修复切换后残留的杂色块）
        for attr in ('chan_top', 'serial_frame', 'udp_frame', 'switch_frame',
                    'channels_canvas', 'channels_container',
                    'color_legend_frame', 'raw_container', 'toolbar_frame'):
            obj = getattr(self, attr, None)
            if obj is not None:
                try:
                    obj.configure(bg=THEME['panel'])
                except Exception:
                    pass




    def _refresh_widgets_colors(self, widget):
        """递归遍历所有控件，刷新背景色和前景色"""
        cls_name = widget.__class__.__name__

        try:
            if cls_name in ("Frame", "Labelframe", "Toplevel"):
                widget.configure(bg=THEME['panel'])
            elif cls_name == "Label":
                if widget is getattr(self, 'status_label', None) or \
                        widget is getattr(self, 'tx_count_label', None):
                    widget.configure(bg=THEME['panel'], fg=THEME['text_dim'])
                else:
                    widget.configure(bg=THEME['panel'], fg=THEME['text'])
            elif cls_name == "Text":
                widget.configure(bg=THEME['plot_bg'], fg=THEME['text'],
                                insertbackground=THEME['accent'],
                                selectbackground=THEME['accent'],
                                selectforeground='#ffffff',
                                highlightbackground=THEME['border'],
                                highlightcolor=THEME['accent'])
            elif cls_name == "Listbox":
                widget.configure(bg=THEME['panel_2'], fg=THEME['text'],
                                selectbackground=THEME['accent'],
                                selectforeground='#ffffff',
                                highlightbackground=THEME['border'])
            elif cls_name == "Canvas":
                widget.configure(bg=THEME['panel'])
            elif cls_name == "Radiobutton":
                widget.configure(
                    bg=THEME['panel'],
                    fg=THEME['text'],
                    selectcolor=THEME['accent'],
                    activebackground=THEME['panel'],
                    activeforeground=THEME['accent'],
                )
            elif cls_name == "Checkbutton":
                widget.configure(
                    bg=THEME['panel'],
                    fg=THEME['text'],
                    selectcolor=THEME['panel_2'],
                    activebackground=THEME['panel'],
                    activeforeground=THEME['accent'],
                )
        except Exception:
            pass

        try:
            for child in widget.winfo_children():
                self._refresh_widgets_colors(child)
        except Exception:
            pass

    # ==========================================================
    # toolbar 深色化
    # ==========================================================
    def _apply_toolbar_dark_style(self):
        def style_widget(w):
            try:
                w.configure(bg=THEME['panel'])
            except Exception:
                pass
            cls_name = w.__class__.__name__
            if cls_name in ("Button", "TkButton"):
                try:
                    w.configure(
                        bg=THEME['panel'],
                        activebackground=THEME['panel_2'],
                        foreground=THEME['text'],
                        activeforeground=THEME['accent'],
                        disabledforeground=THEME['text_dim'],
                        relief='flat', bd=0,
                        highlightthickness=0,
                    )
                except Exception:
                    pass
            elif cls_name in ("Label", "TkLabel"):
                try:
                    w.configure(bg=THEME['panel'], fg=THEME['text'])
                except Exception:
                    pass
            for child in w.winfo_children():
                style_widget(child)

        try:
            style_widget(self.toolbar)
        except Exception:
            pass

        # 2026-11-03 用户反馈：暗色主题下垃圾桶按钮用纯白 fg（THEME['text']）太突兀，
        # 与旁边的灰度位图图标不协调。按工具栏背景亮度判断：暗色背景 → 用柔和的
        # text_dim（暗色主题为 #9da3bb，与图标灰度接近）；浅色主题保持 text 不变。
        try:
            r, g, b = self.root.winfo_rgb(THEME['panel'])[0:3]
            r, g, b = r >> 8, g >> 8, b >> 8
            if (r + g + b) / 3 < 128:
                self.clear_plot_tb_btn.configure(fg=THEME['text_dim'],
                                                 activeforeground=THEME['accent'])
        except Exception:
            pass

        # 隐藏 Back / Forward 按钮
        try:
            def find_buttons(w, result):
                for child in w.winfo_children():
                    cls_name = child.__class__.__name__
                    if cls_name in ("Button", "TkButton"):
                        result.append(child)
                    find_buttons(child, result)
                return result

            all_buttons = find_buttons(self.toolbar, [])
            for idx in (1, 2):
                if idx < len(all_buttons):
                    btn = all_buttons[idx]
                    try:
                        btn.pack_forget()
                    except Exception:
                        try:
                            btn.grid_remove()
                        except Exception:
                            pass
        except Exception:
            pass

    def _make_switch_btn(self, parent, text, value):
        return tk.Radiobutton(
            parent, text=text, variable=self.view_mode_var, value=value,
            command=self._on_view_switch,
            indicatoron=0, width=8,
            bg=THEME['panel_2'], fg=THEME['text'],
            selectcolor=THEME['accent'],
            activebackground=THEME['accent'],
            activeforeground='#ffffff',
            relief='flat', bd=0,
            font=FONT_UI,
            highlightthickness=0,
        )
    def _make_switch_btn_tx(self, parent, text, value):
        """HEX/ASCII 切换按钮（用按钮样式，任意主题都清晰）"""
        return tk.Radiobutton(
            parent, text=text, variable=self.tx_format_var, value=value,
            indicatoron=0, width=6,
            bg=THEME['panel_2'], fg=THEME['text'],
            selectcolor=THEME['accent'],
            activebackground=THEME['accent'],
            activeforeground='#ffffff',
            relief='flat', bd=0,
            font=FONT_UI,
            highlightthickness=0,
        )        

    def _make_radio(self, parent, text, value):
        return tk.Radiobutton(
            parent, text=text, variable=self.tx_format_var, value=value,
            bg=THEME['panel'], fg=THEME['text'],
            selectcolor=THEME['accent'],          # 选中时圆点变成主色（明显）
            activebackground=THEME['panel'],
            activeforeground=THEME['accent'],
            relief='flat', bd=0,
            font=FONT_UI,
            highlightthickness=0,
        )
    def _patch_home_button(self):
        try:
            found = False
            all_buttons = []

            def walk(widget, depth=0):
                nonlocal found
                for child in widget.winfo_children():
                    cls_name = child.__class__.__name__
                    if cls_name in ("Button", "TkButton"):
                        all_buttons.append(child)
                        try:
                            cmd = child.cget('command')
                            if cmd and 'home' in str(cmd).lower():
                                child.config(command=self._home_button_callback)
                                found = True
                        except Exception:
                            pass
                    walk(child, depth + 1)

            walk(self.toolbar)

            if not found and all_buttons:
                all_buttons[0].config(command=self._home_button_callback)
        except Exception:
            pass

    def _home_button_callback(self, *args):
        """点击房子按钮：恢复完整视图（显示所有数据）

        关键点（踩坑记录）：
        用户手动缩放/平移后，matplotlib 会关闭坐标轴的 autoscale 标志。
        此时单独调用 relim() + autoscale_view() 是空操作。
        必须先 set_autoscale_on(True) 重新打开 autoscale 标志。
        """
        try:
            self.ax.set_autoscale_on(True)
            self.ax.relim()
            self.ax.autoscale_view()
            self.canvas.draw_idle()
        except Exception:
            pass

    def _create_channel_widgets(self):
        for w in self.channels_container.winfo_children():
            w.destroy()
        self.channel_widgets = []

        for i in range(self.channel_count):
            color = CHANNEL_COLORS[i % len(CHANNEL_COLORS)]
            chan_data = self.channels[i] if i < len(self.channels) else {'indices': [2, 3], 'sign_bit': -1, 'diff': False}

            chan_frame = tk.Frame(self.channels_container, bg=THEME['panel'])
            chan_frame.grid(row=i, column=0, padx=2, pady=4, sticky=(tk.W, tk.E))
            chan_frame.columnconfigure(6, weight=1)

            # 1) 左侧紧凑单元：[✓]N
            left_box = tk.Frame(chan_frame, bg=THEME['panel'])
            left_box.grid(row=0, column=0, padx=(0, 4), pady=2, sticky=tk.W)

            visible = self.channel_visible[i] if i < len(self.channel_visible) else True
            visible_var = tk.BooleanVar(value=visible)
            visible_check = tk.Checkbutton(
                left_box,
                text="",
                variable=visible_var,
                bg=THEME['panel'],
                fg=color,
                selectcolor=THEME['panel_2'],
                activebackground=THEME['panel'],
                activeforeground=color,
                font=('Microsoft YaHei UI', 10, 'bold'),
                relief='flat', bd=0,
                padx=0, pady=1,
                width=1,
                highlightthickness=0,
                command=lambda idx=i, v=visible_var: self._toggle_channel_visibility(idx, v)
            )
            visible_check.pack(side=tk.LEFT)

            tk.Label(left_box, text=str(i + 1),
                    bg=THEME['panel'], fg=color,
                    font=('Microsoft YaHei UI', 10, 'bold'),
                    padx=0).pack(side=tk.LEFT)

            # 2) Idx
            tk.Label(chan_frame, text="Idx:", bg=THEME['panel'], fg=THEME['text_dim'],
                    font=FONT_LABEL).grid(row=0, column=1, padx=(4, 2), pady=2, sticky=tk.E)
            indices_str = ','.join(str(x) for x in chan_data['indices'])
            indices_var = tk.StringVar(value=indices_str)
            indices_entry = ttk.Entry(chan_frame, textvariable=indices_var, width=6)
            indices_entry.grid(row=0, column=2, padx=(0, 6), pady=2, sticky=tk.W)

            # 3) Sign
            tk.Label(chan_frame, text="Sig:", bg=THEME['panel'], fg=THEME['text_dim'],
                    font=FONT_LABEL).grid(row=0, column=3, padx=(4, 2), pady=2, sticky=tk.E)
            sign_bit_var = tk.StringVar(value=str(chan_data['sign_bit']))
            sign_bit_entry = ttk.Entry(chan_frame, textvariable=sign_bit_var, width=3)
            sign_bit_entry.grid(row=0, column=4, padx=(0, 4), pady=2, sticky=tk.W)

            # 4) Diff
            diff_var = tk.BooleanVar(value=chan_data['diff'])
            diff_check = tk.Checkbutton(
                chan_frame, text="Dif", variable=diff_var,
                bg=THEME['panel'], fg=THEME['text_dim'],
                selectcolor=THEME['panel_2'],
                activebackground=THEME['panel'],
                activeforeground=THEME['accent'],
                relief='flat', bd=0, highlightthickness=0,
                font=FONT_UI,
            )
            diff_check.grid(row=0, column=5, padx=(4, 2), pady=2, sticky=tk.W)

            indices_entry.bind('<FocusOut>', lambda e, sv=sign_bit_var, iv=indices_var: self._auto_update_sign_bit(sv, iv))
            indices_entry.bind('<Return>', lambda e, sv=sign_bit_var, iv=indices_var: self._auto_update_sign_bit(sv, iv))

            self.channel_widgets.append({
                'indices_var': indices_var,
                'sign_bit_var': sign_bit_var,
                'diff_var': diff_var,
                'visible_var': visible_var,
            })

        # 修复1：通道控件重建后立即同步配置快照（主线程上下文，安全）
        self._snapshot_channel_configs()

    def _update_color_legend(self):
        for w in self.color_legend_frame.winfo_children():
            w.destroy()

        for i in range(min(self.channel_count, len(CHANNEL_COLORS))):
            color = CHANNEL_COLORS[i]
            tk.Label(self.color_legend_frame, text=f" {i+1} ", bg=color, fg='#ffffff',
                    font=('Microsoft YaHei UI', 8, 'bold'),
                    padx=4, pady=2).pack(side=tk.LEFT, padx=2)

    def _on_view_switch(self):
        mode = self.view_mode_var.get()
        if mode == "原始":
            self.canvas.get_tk_widget().grid_remove()
            self.toolbar_frame.grid_remove()
            self.raw_container.grid()
        else:
            self.raw_container.grid_remove()
            self.canvas.get_tk_widget().grid()
            self.toolbar_frame.grid()
        self._update_status_mode()

    def _toggle_channel_visibility(self, channel_idx, var):
        if channel_idx < len(self.channel_visible):
            self.channel_visible[channel_idx] = var.get()
            self._redraw_plot()

    def _auto_update_sign_bit(self, sign_bit_var, indices_var):
        try:
            indices_str = indices_var.get().strip()
            if not indices_str:
                return
            indices = self._parse_indices(indices_str)
            byte_count = len(indices)
            if byte_count > 0:
                sign_bit = byte_count * 8 - 1
                sign_bit_var.set(str(sign_bit))
        except (ValueError, IndexError):
            pass

    def _init_plot_lines(self):
        for line in self.channel_lines:
            line.remove()
        for line in self.channel_diff_lines:
            line.remove()
        self.channel_lines = []
        self.channel_diff_lines = []

        for i in range(self.channel_count):
            color = CHANNEL_COLORS[i % len(CHANNEL_COLORS)]
            line, = self.ax.plot([], [], color=color, linewidth=1.5, marker='.', markersize=3,
                            label=f'CH{i+1}')
            self.channel_lines.append(line)

            diff_line, = self.ax.plot([], [], color=color, linewidth=1, linestyle='--', alpha=0.7,
                                    label='_nolegend_')
            self.channel_diff_lines.append(diff_line)
            diff_line.set_visible(False)

        ncol = max(1, min(self.channel_count, 8))
        leg = self.ax.legend(loc='upper left', fontsize=8,
                            ncol=ncol,
                            framealpha=0.85,
                            facecolor=THEME['panel'],
                            edgecolor=THEME['border'],
                            labelcolor=THEME['text'],
                            borderpad=0.4,
                            columnspacing=0.9,
                            handlelength=1.6,
                            handletextpad=0.4)


    def _deep_scan_ports(self):
        """修复3：深度扫描（并发强开 COM1~COM30 探测端口）。
        仅在用户点击"Deep Scan"按钮时执行；正被其他软件占用的端口会被跳过，不会干扰。"""
        self.refresh_ports(deep_scan=True)

    def refresh_ports(self, deep_scan=False):
        """刷新串口列表
        - pyserial 枚举（带描述信息）
        - 读注册表（瞬时，能拿到 VSPD 虚拟串口）
        - 并发扫描 COM1~COM30（兜底，修复3：改为仅用户点击"深度扫描"按钮时执行）
        """
        port_list = []
        seen = set()

        # 1) pyserial 枚举
        try:
            ports = serial.tools.list_ports.comports()
            for p in ports:
                desc = (p.description or "").strip()
                if desc and desc != "n/a":
                    label = f"{p.device} #{desc}"
                else:
                    label = p.device
                port_list.append(label)
                seen.add(p.device.upper())
        except Exception:
            pass

        # 2) 读注册表（瞬时，能拿到 VSPD 虚拟串口）
        # 修复3：用 with 语法管理注册表句柄，异常时保证释放
        try:
            import winreg
            with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,
                                r"HARDWARE\DEVICEMAP\SERIALCOMM") as key:
                i = 0
                while True:
                    try:
                        name, value, _ = winreg.EnumValue(key, i)
                        if value.upper() not in seen:
                            port_list.append(value)
                            seen.add(value.upper())
                        i += 1
                    except OSError:
                        break
        except Exception:
            pass

        # 3) 并发扫描 COM1~COM30（兜底）
        # 修复3：该扫描会短暂强开端口，可能干扰正被占用的串口，
        # 移出默认刷新流程，仅在用户点击"深度扫描"按钮时执行
        if deep_scan:
            try:
                from concurrent.futures import ThreadPoolExecutor

                def try_open(i):
                    name = f"COM{i}"
                    if name.upper() in seen:
                        return None
                    try:
                        s = serial.Serial(name, timeout=0.005)
                        s.close()
                        return name
                    except Exception:
                        return None

                with ThreadPoolExecutor(max_workers=10) as executor:
                    results = executor.map(try_open, range(1, 31))
                    for r in results:
                        if r and r.upper() not in seen:
                            port_list.append(r)
                            seen.add(r.upper())
            except Exception:
                pass

        # 修复3：保持原选中项（若仍在新列表中），否则才选第 0 项；列表为空时清空选中
        prev_name = self._extract_port_name(self.port_var.get()).upper()
        self.port_combo['values'] = port_list
        if port_list:
            plain_names = [self._extract_port_name(p).upper() for p in port_list]
            if prev_name and prev_name in plain_names:
                self.port_var.set(port_list[plain_names.index(prev_name)])
            else:
                self.port_combo.current(0)
        else:
            self.port_var.set("")

    def _extract_port_name(self, port_str):
        if not port_str:
            return ""
        return port_str.split('#', 1)[0].strip()

    def _validate_channel_indices(self, channel_configs, frame_length=None, samples_per_packet=None):
        """修复6：校验每个激活通道的字节索引是否越界。
        samples_per_packet>1 时，后续样本按 sample_start+sample_size 平移，
        最坏情况为最后一个样本：max(indices) + (spp-1)*sample_size。
        越界则弹出错误框指明通道与索引，返回 False 并中止应用。"""
        if frame_length is None:
            frame_length = self.frame_length
        if samples_per_packet is None:
            samples_per_packet = self.samples_per_packet

        active = [cfg for cfg in channel_configs
                  if cfg.get('active') and cfg.get('indices')]
        if not active:
            return True

        # 大包模式下先算出所有激活通道的平移步长 sample_size
        sample_size = 0
        if samples_per_packet > 1:
            all_indices = []
            for cfg in active:
                all_indices.extend(cfg['indices'])
            if all_indices:
                sample_size = max(all_indices) - min(all_indices) + 1

        for i, cfg in enumerate(channel_configs):
            if not cfg.get('active') or not cfg.get('indices'):
                continue
            max_required = max(cfg['indices']) + (samples_per_packet - 1) * sample_size + 1
            if max_required > frame_length:
                if samples_per_packet > 1:
                    messagebox.showerror(
                        "Config Error",
                        f"通道 CH{i+1} 字节索引越界：\n"
                        f"最大索引 {max(cfg['indices'])}，Samples/Pkt={samples_per_packet} "
                        f"时最后一个样本的最坏索引为 {max_required - 1}，\n"
                        f"超出帧长 {frame_length}（有效索引范围 0 ~ {frame_length - 1}）。\n"
                        f"请修正该通道的 Idx 或帧配置后重试。")
                else:
                    messagebox.showerror(
                        "Config Error",
                        f"通道 CH{i+1} 字节索引越界：\n"
                        f"最大索引 {max(cfg['indices'])} 超出帧长 {frame_length}"
                        f"（有效索引范围 0 ~ {frame_length - 1}）。\n"
                        f"请修正该通道的 Idx 后重试。")
                return False
        return True

    def apply_frame_config(self):
        try:
            header_str = self.header_var.get().replace(' ', '').replace('\n', '')
            new_header = bytes.fromhex(header_str)
            new_length = int(self.length_var.get())
            new_order = self.order_var.get()
            new_spp = int(self.samples_var.get())
            if new_spp < 1:
                new_spp = 1
            new_max_points = int(self.max_pts_var.get())

            # 修复6：先校验激活通道的索引是否越界（用即将应用的新帧参数），越界则中止应用
            if not self._validate_channel_indices(self._read_channel_configs_from_ui(),
                                                  frame_length=new_length,
                                                  samples_per_packet=new_spp):
                return

            self.frame_header = new_header
            self.frame_length = new_length
            self.byte_order = new_order
            self.samples_per_packet = new_spp
            self.max_points = new_max_points

            for i in range(len(self.channel_values)):
                self.channel_values[i] = deque(self.channel_values[i], maxlen=self.max_points)
                self.channel_diff_values[i] = deque(self.channel_diff_values[i], maxlen=self.max_points)

            # 修复1：主线程上下文，同步配置快照供接收线程使用
            self._snapshot_channel_configs()

            messagebox.showinfo("Config", "Frame configuration applied")
        except Exception as e:
            messagebox.showerror("Config Error", str(e))

    def _on_interface_change(self, event=None):
        if self.is_connected:
            self.disconnect()

        iface = self.interface_var.get()
        self.interface_type = iface.lower()

        if self.interface_type == "serial":
            self.serial_frame.grid()
            self.udp_frame.grid_remove()
        elif self.interface_type == "udp":
            self.serial_frame.grid_remove()
            self.udp_frame.grid()

    def apply_channel_config(self):
        try:
            count = int(self.channel_count_var.get())
            if count < 1:
                count = 1
            if count > MAX_CHANNELS:
                count = MAX_CHANNELS
                self.channel_count_var.set(str(count))

            old_count = len(self.channel_widgets)
            old_configs = []
            for i in range(old_count):
                w = self.channel_widgets[i]
                indices_str = w['indices_var'].get()
                indices = self._parse_indices(indices_str) if indices_str.strip() else []
                sign_bit = int(w['sign_bit_var'].get()) if w['sign_bit_var'].get().strip() else -1
                diff = w['diff_var'].get()
                old_configs.append({'indices': indices, 'sign_bit': sign_bit, 'diff': diff})

            self.channel_count = count

            new_channels = []
            new_values = []
            new_diff_values = []
            new_visible = []

            for i in range(count):
                if i < len(old_configs) and old_configs[i]['indices']:
                    cfg = old_configs[i]
                else:
                    start_idx = 2 + i * 2
                    cfg = {'indices': [start_idx, start_idx + 1], 'sign_bit': -1, 'diff': False}
                new_channels.append(cfg)
                new_values.append(deque(maxlen=self.max_points))
                new_diff_values.append(deque(maxlen=self.max_points))
                if i < len(self.channel_visible):
                    new_visible.append(self.channel_visible[i])
                else:
                    new_visible.append(True)

            # 修复6：校验新通道配置的索引是否越界（全部按激活处理），越界则中止应用
            check_list = [{'indices': c['indices'], 'sign_bit': c['sign_bit'],
                           'diff': c['diff'], 'active': True} for c in new_channels]
            if not self._validate_channel_indices(check_list):
                return

            self.channels = new_channels
            self.channel_values = new_values
            self.channel_diff_values = new_diff_values
            self.channel_visible = new_visible

            self._create_channel_widgets()
            self._update_color_legend()
            self._init_plot_lines()
            self._redraw_plot()

            messagebox.showinfo("Config", f"Channel configuration applied ({count} channels)")
        except Exception as e:
            messagebox.showerror("Config Error", str(e))

    def _parse_indices(self, indices_str):
        return [int(x.strip()) for x in re.split(r'[,\s，、]+', indices_str) if x.strip()]

    def _read_channel_configs_from_ui(self):
        """读取通道配置（必须只在主线程调用，禁止在接收线程中使用）"""
        configs = []
        for i, w in enumerate(self.channel_widgets):
            indices_str = w['indices_var'].get().strip()
            if indices_str:
                try:
                    indices = self._parse_indices(indices_str)
                    sign_bit = int(w['sign_bit_var'].get()) if w['sign_bit_var'].get().strip() else -1
                    diff = w['diff_var'].get()
                    configs.append({
                        'indices': indices,
                        'sign_bit': sign_bit,
                        'diff': diff,
                        'active': True
                    })
                except ValueError:
                    configs.append({'indices': [], 'sign_bit': -1, 'diff': False, 'active': False})
            else:
                configs.append({'indices': [], 'sign_bit': -1, 'diff': False, 'active': False})
        return configs

    def _snapshot_channel_configs(self):
        """修复1：在主线程上下文中把通道配置快照下来，存入 self.active_channel_configs。
        接收线程（_process_buffer）只读快照，不再跨线程读 Tkinter 变量。"""
        self.active_channel_configs = self._read_channel_configs_from_ui()
        return self.active_channel_configs

    def sign_extend(self, value, sign_bit_pos):
        if sign_bit_pos < 0:
            return value
        sign_mask = 1 << sign_bit_pos
        if value & sign_mask:
            extend_mask = ~((1 << (sign_bit_pos + 1)) - 1)
            value = value | extend_mask
        return value

    def _collect_raw_frame(self, frame_seq, frame_bytes):
        self.raw_frame_buffer.append((frame_seq, frame_bytes))
        self._raw_update_pending = True

    def _extract_value_from_frame(self, frame, indices, sign_bit):
        if not indices:
            return None
        try:
            value = 0
            if self.byte_order == "big":
                for idx in indices:
                    value = (value << 8) | frame[idx]
            else:
                for idx in reversed(indices):
                    value = (value << 8) | frame[idx]
            value = self.sign_extend(value, sign_bit)
            return value
        except IndexError:
            return None

    def _append_channel_value(self, ch_idx, val):
        """修复8：追加一个新样本值，并在收到新值时直接做一阶差分增量计算
        （diffs[i] = values[i] - values[i-1]，与原全量重算语义一致），
        避免绘图时每个刷新周期对整条曲线全量重算。"""
        values = self.channel_values[ch_idx]
        last = values[-1] if len(values) > 0 else None
        values.append(val)
        if last is not None:
            self.channel_diff_values[ch_idx].append(val - last)

    def _rebuild_diff_values(self):
        """修复8：离线分析/配置变更场景下从 channel_values 全量重建差分队列。
        差分队列的 maxlen 与 channel_values 保持一致（初始化/重建时同步设定）。"""
        for i in range(len(self.channel_values)):
            vals = list(self.channel_values[i])
            diffs = self.channel_diff_values[i]
            diffs.clear()
            for j in range(1, len(vals)):
                diffs.append(vals[j] - vals[j - 1])

    def toggle_connection(self):
        if self.is_connected:
            self.disconnect()
        else:
            self.connect()

    def connect(self):
        try:
            self.apply_frame_config()
            # 修复1：在主线程上下文中生成通道配置快照，供接收线程使用
            self._snapshot_channel_configs()

            self.stop_event.clear()
            self.rx_buffer.clear()
            self.frame_count = 0    # 原始帧计数（帧序号）
            self.sample_count = 0   # 样本计数（CSV 提取数据的 Frame 列）

            for i in range(self.channel_count):
                self.channel_values[i].clear()
                self.channel_diff_values[i].clear()   # 修复8：差分队列清空时机与 channel_values 一致

            self._clear_raw_view()

            self._extracted_save_buffer = []
            self._extracted_save_frame_start = 0

            if self.interface_type == "serial":
                port = self.port_var.get()
                if not port:
                    messagebox.showerror("Error", "Select a port first")
                    return

                port = self._extract_port_name(port)

                try:
                    self.serial_port = serial.Serial(
                        port=port,
                        baudrate=int(self.baud_var.get()),
                        timeout=0.1
                    )
                except serial.SerialException:
                    messagebox.showerror("Serial Error",
                        "Cannot open serial port '" + port + "'.\n"
                        "Please check:\n"
                        "1. The port exists (click Refresh to update list)\n"
                        "2. The port is not occupied by other software\n"
                        "3. The USB cable is properly connected")
                    return
                except ValueError:
                    messagebox.showerror("Serial Error", "Invalid baud rate setting")
                    return

                self.is_connected = True
                # 1.2 修改（功能4）：按钮文本经 tr() 取当前语言文案
                self.connect_btn.config(text=self.tr('disconnect'))
                self.status_var.set(f"Serial: {port} @ {self.baud_var.get()} | {self.channel_count} ch")
                self.receive_thread = threading.Thread(target=self.receive_loop, daemon=True)
                self.receive_thread.start()

            elif self.interface_type == "udp":
                local_ip = self.udp_local_ip_var.get().strip() or "0.0.0.0"
                try:
                    local_port = int(self.udp_local_port_var.get())
                except ValueError:
                    messagebox.showerror("UDP Error", "Invalid local port number")
                    return

                try:
                    self.udp_socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                    self.udp_socket.bind((local_ip, local_port))
                    self.udp_socket.settimeout(0.1)
                except OSError as e:
                    messagebox.showerror("UDP Error", "Cannot bind to " + local_ip + ":" + str(local_port) + ":\n" + str(e))
                    if self.udp_socket:
                        try:
                            self.udp_socket.close()
                        except:
                            pass
                        self.udp_socket = None
                    return

                remote_ip = self.udp_remote_ip_var.get().strip()
                remote_port_str = self.udp_remote_port_var.get().strip()
                if remote_ip and remote_port_str:
                    try:
                        self.udp_remote_addr = (remote_ip, int(remote_port_str))
                    except ValueError:
                        messagebox.showerror("UDP Error", "Invalid remote port number")
                        self.udp_socket.close()
                        self.udp_socket = None
                        return
                else:
                    self.udp_remote_addr = None

                self.is_connected = True
                # 1.2 修改（功能4）：按钮文本经 tr() 取当前语言文案
                self.connect_btn.config(text=self.tr('disconnect'))
                self.status_var.set(f"UDP: port {local_port} | {self.channel_count} ch")
                self.receive_thread = threading.Thread(target=self.udp_receive_loop, daemon=True)
                self.receive_thread.start()

            self._start_plot_update()
        except Exception as e:
            messagebox.showerror("Connection Error", str(e))

    def disconnect(self):
        self.stop_event.set()
        if getattr(self, 'serial_port', None) and self.serial_port.is_open:
            self.serial_port.close()
        udp_sock = getattr(self, 'udp_socket', None)
        if udp_sock:
            try:
                udp_sock.close()
            except:
                pass
            self.udp_socket = None
        self.is_connected = False
        # 1.2 修改（功能4）：按钮文本经 tr() 取当前语言文案
        self.connect_btn.config(text=self.tr('connect'))
        self.status_var.set("Disconnected")

        # 修复7：提取数据 CSV 写盘逻辑抽成公共方法，与 _update_plot 共用
        self._flush_extracted_save()

    def _flush_extracted_save(self):
        """修复7：把提取数据缓冲写入 CSV（含表头，含 _extracted_header_written 逻辑）。
        disconnect() 和 _update_plot() 共用本方法，行为保持一致。"""
        if not (self._extracted_save_buffer and self.extracted_save_file):
            return
        buffer = self._extracted_save_buffer
        self._extracted_save_buffer = []
        try:
            with open(self.extracted_save_file, 'a') as f:
                if not self._extracted_header_written:
                    # 主线程调用时可读快照；快照为空（未连接过）则退回读 UI
                    channel_configs = self.active_channel_configs if self.active_channel_configs \
                        else self._read_channel_configs_from_ui()
                    active_channels = [i for i, cfg in enumerate(channel_configs) if cfg['active']]
                    header = "Frame," + ",".join(f"CH{i+1}" for i in active_channels) + "\n"
                    f.write(header)
                    self._extracted_header_written = True

                # 修复5：CSV 提取数据的 Frame 列使用 sample_count（样本级别计数）
                for frame_num, values in buffer:
                    line = f"{frame_num}," + ",".join(str(v) for v in values) + "\n"
                    f.write(line)
        except Exception:
            pass

    def _schedule_link_lost(self, reason):
        """修复4：接收线程异常退出前，调度主线程方法提示用户（线程不能静默死亡）"""
        try:
            self.root.after(0, self._on_link_lost, reason)
        except Exception:
            pass

    def _on_link_lost(self, reason):
        """修复4：接收线程异常退出时由主线程调用（root.after 调度）：
        弹窗提示连接断开原因，并复用 disconnect() 复位按钮和状态栏。
        仅在 is_connected 为 True 时执行，避免与正常断开流程重入。"""
        if not self.is_connected:
            return
        messagebox.showwarning("Connection Lost", f"连接已断开：{reason}")
        self.disconnect()

    def send_data(self):
        if not self.is_connected:
            messagebox.showerror("Error", "Not connected")
            return

        try:
            data_str = self.tx_data_var.get()
            if self.tx_format_var.get() == "hex":
                hex_str = re.sub(r'[\s\r\n]+', '', data_str)
                data = bytes.fromhex(hex_str)
            else:
                data = data_str.encode('ascii', errors='replace')
        except ValueError as e:
            messagebox.showerror("Error", f"Invalid hex data: {e}")
            return

        try:
            if self.interface_type == "serial":
                if not self.serial_port or not self.serial_port.is_open:
                    messagebox.showerror("Error", "Serial port not connected")
                    return
                self.serial_port.write(data)

            elif self.interface_type == "udp":
                if not self.udp_socket:
                    messagebox.showerror("Error", "UDP not connected")
                    return
                if not self.udp_remote_addr:
                    messagebox.showerror("Error",
                        "No remote address.\n"
                        "Set Remote IP/Port or receive data first.")
                    return
                self.udp_socket.sendto(data, self.udp_remote_addr)

            self.tx_count += 1
            self.tx_count_var.set(f"TX: {self.tx_count}")
            self.status_var.set(
                f"TX sent {len(data)} bytes: {data_str[:40]}{'...' if len(data_str) > 40 else ''}"
            )
            fmt = self.tx_format_var.get()
            entry = (fmt, data_str)
            if not self.send_history or self.send_history[-1] != entry:
                self.send_history.append(entry)
                if len(self.send_history) > self.max_history:
                    self.send_history.pop(0)
        except Exception as e:
            messagebox.showerror("Send Error", str(e))

    def clear_tx_log(self):
        self.tx_count = 0
        self.tx_count_var.set("TX: 0")

    def _apply_dialog_icon(self, dlg):
        """2026-11-03：子窗口（Toplevel）默认继承 Tk 羽毛图标，
        这里统一设置成与主窗口相同的 app.ico。找不到图标文件时静默跳过。"""
        try:
            icon = getattr(self, '_icon_path', None)
            if icon and os.path.isfile(icon):
                dlg.iconbitmap(icon)
        except Exception:
            pass

    def show_send_history(self):
        if not self.send_history:
            messagebox.showinfo("History", "No send history yet.")
            return

        dlg = tk.Toplevel(self.root)
        self._apply_dialog_icon(dlg)
        dlg.title("Send History")
        dlg.geometry("500x400")
        dlg.transient(self.root)
        dlg.grab_set()
        dlg.configure(bg=THEME['bg'])

        frame = tk.Frame(dlg, padx=10, pady=10, bg=THEME['bg'])
        frame.pack(fill=tk.BOTH, expand=True)

        tk.Label(frame, text=f"Last {len(self.send_history)} commands (newest at bottom):",
                bg=THEME['bg'], fg=THEME['text_dim'], font=FONT_UI).pack(anchor=tk.W)

        list_frame = tk.Frame(frame, bg=THEME['bg'])
        list_frame.pack(fill=tk.BOTH, expand=True, pady=5)

        scrollbar = ttk.Scrollbar(list_frame)
        scrollbar.pack(side=tk.RIGHT, fill=tk.Y)

        listbox = tk.Listbox(list_frame, yscrollcommand=scrollbar.set,
                            font=FONT_MONO,
                            bg=THEME['panel_2'], fg=THEME['text'],
                            selectbackground=THEME['accent'],
                            selectforeground='#ffffff',
                            relief='flat', bd=0, highlightthickness=1,
                            highlightbackground=THEME['border'],
                            highlightcolor=THEME['accent'])
        listbox.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        scrollbar.config(command=listbox.yview)

        for i, (fmt, data) in enumerate(reversed(self.send_history)):
            preview = data[:60].replace('\n', ' ')
            if len(data) > 60:
                preview += "..."
            listbox.insert(tk.END, f"[{fmt.upper()}] {preview}")

        def on_select(event=None):
            selection = listbox.curselection()
            if not selection:
                return
            idx = len(self.send_history) - 1 - selection[0]
            fmt, data = self.send_history[idx]
            self.tx_format_var.set(fmt)
            self.tx_data_var.set(data)
            dlg.destroy()

        def on_send(event=None):
            selection = listbox.curselection()
            if not selection:
                return
            idx = len(self.send_history) - 1 - selection[0]
            fmt, data = self.send_history[idx]
            self.tx_format_var.set(fmt)
            self.tx_data_var.set(data)
            dlg.destroy()
            self.send_data()

        listbox.bind('<Double-Button-1>', on_send)

        btn_frame = tk.Frame(frame, bg=THEME['bg'])
        btn_frame.pack(fill=tk.X, pady=5)

        ttk.Button(btn_frame, text="Load to Input", command=on_select).pack(side=tk.LEFT, padx=5)
        ttk.Button(btn_frame, text="Send Immediately", command=on_send).pack(side=tk.LEFT, padx=5)
        ttk.Button(btn_frame, text="Cancel", command=dlg.destroy).pack(side=tk.RIGHT, padx=5)

        if self.send_history:
            listbox.selection_set(0)
            listbox.activate(0)

    def show_help(self):
        dlg = tk.Toplevel(self.root)
        self._apply_dialog_icon(dlg)
        dlg.title("帮助 - 二进制帧分析工具")
        dlg.geometry("700x600")
        dlg.transient(self.root)
        dlg.configure(bg=THEME['bg'])

        frame = tk.Frame(dlg, padx=10, pady=10, bg=THEME['bg'])
        frame.pack(fill=tk.BOTH, expand=True)

        # 用 tk.Text + 手动 Scrollbar 代替 ScrolledText，避免颜色被覆盖
        text_wrap = tk.Frame(frame, bg=THEME['bg'])
        text_wrap.pack(fill=tk.BOTH, expand=True)

        scrollbar = ttk.Scrollbar(text_wrap, orient=tk.VERTICAL)
        scrollbar.pack(side=tk.RIGHT, fill=tk.Y)

        text = tk.Text(text_wrap, wrap=tk.WORD,
                    font=FONT_UI,
                    bg=THEME['panel_2'], fg=THEME['text'],
                    insertbackground=THEME['accent'],
                    selectbackground=THEME['accent'],
                    selectforeground='#ffffff',
                    relief='flat', bd=0,
                    highlightthickness=1,
                    highlightbackground=THEME['border'],
                    yscrollcommand=scrollbar.set)
        text.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        scrollbar.config(command=text.yview)

        help_text = """
二进制帧分析工具(帧析助手) 使用说明
作者: RhycWu
邮箱: gykjwrq@163.com

【首次使用】
    python版: pip install pyserial matplotlib
    Windows版: 双击exe

【接口选择】
    串口（Serial）：
    - 端口（Port）：选择串口号（点击 Refresh 刷新列表）
    - 波特率（Baud）：9600 ~ 3000000 可选

    网口（UDP）：
    - 本地 IP：绑定的网卡地址（默认 0.0.0.0）
    - 本地端口：监听的 UDP 端口
    - 远程 IP / 远程端口：发送目标地址（可选）

【帧配置】
    - 帧头：标记帧开始的十六进制字节，如 EB 90
    - 帧长：每帧的总字节数
    - 字节序：big / little
    - 每包采样数：大包模式下的采样点数
    - 最大显示点数：曲线保留的最近点数

【通道配置】
    - 通道数量：1 ~ 8 路
    - Idx：通道数据在帧中的字节位置（逗号分隔，从 0 开始）
    - Sign：符号位位置（-1 表示无符号）
    - Diff：启用一阶差分，用虚线绘制

【数据发送】
    - HEX：发送十六进制字节
    - ASCII：发送文本
    - History：查看发送历史

【数据保存】
    - Save Raw Frames：自动保存所有原始数据（TXT 十六进制文本）
    - Save Samples：自动保存提取的通道数值（CSV）

【离线分析】
    - Plot Raw Data：加载十六进制数据文件并绘图
    - Plot from Clipboard：分析剪贴板中的十六进制数据并绘图
"""

        # 关键：先正常状态插入，再禁用
        text.insert(tk.END, help_text)
        text.config(state=tk.DISABLED)


        btn_frame = tk.Frame(dlg, padx=10, pady=10, bg=THEME['bg'])
        btn_frame.pack(fill=tk.X)
        ttk.Button(btn_frame, text="关闭", command=dlg.destroy).pack(side=tk.RIGHT)

    def toggle_auto_save(self):
        self.auto_save_enabled = self.auto_save_var.get()
        if self.auto_save_enabled and not self.auto_save_file:
            self.select_save_file()
            if not self.auto_save_file:
                self.auto_save_var.set(False)
                self.auto_save_enabled = False

    @staticmethod
    def _short_path(path, maxlen=20):
        """路径过长时只显示尾部（保留文件名），前面用 … 省略。
        完整路径通过悬停提示查看——避免长路径换行撑爆 Data Log 组布局。"""
        if len(path) <= maxlen:
            return path
        return "…" + path[-(maxlen - 1):]

    def _attach_tooltip(self, widget, get_text):
        """通用悬停提示：鼠标悬停在控件上时弹小窗显示 get_text() 的返回值，移开即消失。
        颜色随当前主题（THEME 在弹出时取值）。"""
        tip = {'win': None}

        def enter(_):
            text = get_text()
            if not text:
                return
            win = tk.Toplevel(widget)
            win.wm_overrideredirect(True)
            win.attributes('-topmost', True)
            wx = widget.winfo_rootx() + 8
            wy = widget.winfo_rooty() + widget.winfo_height() + 4
            win.wm_geometry(f"+{wx}+{wy}")
            tk.Label(win, text=text, bg=THEME['panel'], fg=THEME['text'],
                     font=FONT_UI, padx=6, pady=3, justify=tk.LEFT,
                     highlightthickness=1,
                     highlightbackground=THEME['border']).pack()
            tip['win'] = win

        def leave(_):
            if tip['win'] is not None:
                tip['win'].destroy()
                tip['win'] = None

        widget.bind('<Enter>', enter)
        widget.bind('<Leave>', leave)

    def _attach_path_tooltip(self, label, get_path):
        """鼠标悬停在路径标签上时，弹小窗显示完整路径，移开即消失。"""
        self._attach_tooltip(label, get_path)

    def select_save_file(self):
        filepath = filedialog.asksaveasfilename(
            title="Select Save File",
            defaultextension=".txt",
            filetypes=[("Text files", "*.txt"), ("Hex files", "*.hex"), ("All files", "*.*")]
        )
        if filepath:
            self.auto_save_file = filepath
            # 1.2 修复：长路径只显示尾部，悬停看完整路径，防止撑爆布局
            self.save_file_label.config(text=self._short_path(filepath), foreground=THEME['text'])
            try:
                with open(filepath, 'w') as f:
                    pass
            except Exception as e:
                messagebox.showerror("Error", f"Cannot create file: {e}")

    def toggle_extracted_save(self):
        self.extracted_save_enabled = self.extracted_save_var.get()
        if self.extracted_save_enabled and not self.extracted_save_file:
            self.select_extracted_save_file()
            if not self.extracted_save_file:
                self.extracted_save_var.set(False)
                self.extracted_save_enabled = False

    def select_extracted_save_file(self):
        filepath = filedialog.asksaveasfilename(
            title="Select Extracted Data CSV File",
            defaultextension=".csv",
            filetypes=[("CSV files", "*.csv"), ("Text files", "*.txt"), ("All files", "*.*")]
        )
        if filepath:
            self.extracted_save_file = filepath
            # 1.2 修复：长路径只显示尾部，悬停看完整路径，防止撑爆布局
            self.extracted_save_label.config(text=self._short_path(filepath), foreground=THEME['text'])
            self._extracted_save_buffer = []
            self._extracted_save_frame_start = self.sample_count
            try:
                import os
                self._extracted_header_written = os.path.exists(filepath) and os.path.getsize(filepath) > 0
            except Exception:
                self._extracted_header_written = False

    def receive_loop(self):
        while not self.stop_event.is_set() and self.serial_port and self.serial_port.is_open:
            try:
                if self.serial_port.in_waiting > 0:
                    data = self.serial_port.read(self.serial_port.in_waiting)
                    self.rx_buffer.extend(data)
                    self._save_pending.extend(data)
                    self.rx_total_bytes += len(data)
                    self._process_buffer()
                time.sleep(0.005)
            except Exception as e:
                # 修复4：线程异常退出前调度主线程提示，避免静默死亡
                # （正常断开时 stop_event 已置位，不会触发提示）
                if not self.stop_event.is_set():
                    self._schedule_link_lost(str(e))
                break

    def udp_receive_loop(self):
        while not self.stop_event.is_set() and self.udp_socket:
            try:
                data, addr = self.udp_socket.recvfrom(65536)
                if data:
                    self.rx_buffer.extend(data)
                    self._save_pending.extend(data)
                    self.rx_total_bytes += len(data)
                    if not self.udp_remote_addr:
                        self.udp_remote_addr = addr
                    self._process_buffer()
            except socket.timeout:
                continue
            except OSError as e:
                if self.stop_event.is_set() or not self.udp_socket:
                    break
                # 修复4：连接异常断开，调度主线程提示
                self._schedule_link_lost(str(e))
                break
            except Exception as e:
                # 修复4：线程异常退出前调度主线程提示，避免静默死亡
                if not self.stop_event.is_set():
                    self._schedule_link_lost(str(e))
                break

    def _process_buffer(self):
        """帧同步与数值提取（在接收线程中运行）。
        修复1：本方法只读 self.active_channel_configs 配置快照，
        不再调用 _read_channel_configs_from_ui()（跨线程读 Tkinter 变量不安全）。
        修复5：frame_count 为原始帧级别计数（每成功解析一帧 +1，用于原始视图标注与状态栏），
        sample_count 为样本级别计数（CSV 提取数据的 Frame 列）。"""
        header_len = len(self.frame_header)
        channel_configs = self.active_channel_configs
        active_channels = [i for i, cfg in enumerate(channel_configs) if cfg['active']]

        sample_start = 0
        sample_size = 0
        if self.samples_per_packet > 1 and active_channels:
            all_indices = []
            for i in active_channels:
                all_indices.extend(channel_configs[i]['indices'])
            if all_indices:
                min_idx = min(all_indices)
                max_idx = max(all_indices)
                sample_start = min_idx
                sample_size = max_idx - min_idx + 1

        while len(self.rx_buffer) >= self.frame_length:
            idx = self.rx_buffer.find(self.frame_header)
            if idx == -1:
                if len(self.rx_buffer) > header_len - 1:
                    del self.rx_buffer[-(header_len - 1):]
                break
            if idx > 0:
                del self.rx_buffer[:idx]
            if len(self.rx_buffer) < self.frame_length:
                break

            frame = bytes(self.rx_buffer[:self.frame_length])
            del self.rx_buffer[:self.frame_length]

            # 修复5：原始视图标注使用帧序号 frame_count；成功解析一帧后帧计数 +1
            self._collect_raw_frame(self.frame_count, frame)
            self.frame_count += 1

            num_samples = self.samples_per_packet if self.samples_per_packet > 1 else 1

            for s in range(num_samples):
                sample_values = []
                sample_ok = True

                for i, cfg in enumerate(channel_configs):
                    if cfg['active'] and i < self.channel_count:
                        if self.samples_per_packet > 1:
                            adjusted_indices = [idx + s * sample_size for idx in cfg['indices']]
                        else:
                            adjusted_indices = cfg['indices']

                        val = self._extract_value_from_frame(frame, adjusted_indices, cfg['sign_bit'])
                        if val is not None:
                            # 修复8：追加新值的同时做一阶差分增量计算
                            self._append_channel_value(i, val)
                            sample_values.append(val)
                        else:
                            sample_ok = False

                if sample_ok and self.extracted_save_enabled and len(sample_values) == len(active_channels):
                    # 修复5：CSV 提取数据的 Frame 列使用样本级别计数 sample_count
                    self._extracted_save_buffer.append((self.sample_count, sample_values))

                self.sample_count += 1

    def _update_raw_view(self):
        """修复2：用帧序号 seq 追踪增量刷新（替代原来基于 len(buf) 的索引比较，
        deque(maxlen=1000) 满后 len 恒定会导致全量清空重画）。
        检测到 seq 回绕/重连（缓冲中出现小于已显示最大 seq 的帧）时清空重画。
        Text 控件行数上限裁剪逻辑保留。"""
        if self.view_mode_var.get() != "原始" or not self._raw_update_pending:
            return
        buf = self.raw_frame_buffer
        if len(buf) == 0:
            self._raw_update_pending = False
            return
        try:
            last_seq = self._raw_last_displayed_seq
            # 检测 seq 回绕/重连：缓冲中最小的 seq 比已显示的最大 seq 还小 → 计数复位，清空重画
            if last_seq is not None and buf[0][0] < last_seq:
                last_seq = None
            self.raw_text.config(state=tk.NORMAL)
            if last_seq is None:
                self.raw_text.delete("1.0", tk.END)
                new_items = list(buf)
            else:
                new_items = [(seq, data) for seq, data in buf if seq > last_seq]
            for seq, data in new_items:
                hex_str = ' '.join(f'{b:02X}' for b in data)
                self.raw_text.insert(tk.END, f"{seq}: {hex_str}\n")
            if new_items:
                self._raw_last_displayed_seq = max(seq for seq, _ in new_items)
            # 行数上限裁剪逻辑保留
            total_lines = int(self.raw_text.index('end-1c').split('.')[0])
            if total_lines > 1000:
                self.raw_text.delete("1.0", f"{total_lines - 1000}.0")
            self.raw_text.see(tk.END)
            self.raw_text.config(state=tk.DISABLED)
        except Exception:
            pass
        self._raw_update_pending = False

    def _update_status_mode(self):
        mode = self.view_mode_var.get()
        current = self.status_var.get()
        for tag in [" [绘图]", " [原始]"]:
            current = current.replace(tag, "")
        self.status_var.set(current + f" [{mode}]")

    def _start_plot_update(self):
        if self.is_connected:
            self._update_plot()
            self._update_raw_view()
            self.root.after(100, self._start_plot_update)

    def _update_plot(self):
        if self.auto_save_enabled and self._save_pending and self.auto_save_file:
            data = bytes(self._save_pending)
            self._save_pending.clear()
            hex_str = ' '.join(f'{b:02X}' for b in data)
            try:
                with open(self.auto_save_file, 'a') as f:
                    f.write(hex_str + ' ')
            except Exception:
                pass

        # 修复7：提取数据 CSV 写盘逻辑与 disconnect() 共用同一方法
        self._flush_extracted_save()

        # 修复1：主线程运行，为一致性直接使用配置快照（与接收线程同源）
        channel_configs = self.active_channel_configs

        has_data = False
        for i in range(self.channel_count):
            cfg = channel_configs[i] if i < len(channel_configs) else {'active': False, 'diff': False}
            visible = self.channel_visible[i] if i < len(self.channel_visible) else True

            if cfg['active'] and len(self.channel_values[i]) > 0 and visible:
                has_data = True
                vals = list(self.channel_values[i])
                x = list(range(len(vals)))
                self.channel_lines[i].set_data(x, vals)
                self.channel_lines[i].set_visible(True)

                # 修复8：差分曲线直接使用增量维护的 channel_diff_values，不再全量重算
                if cfg['diff'] and len(self.channel_diff_values[i]) > 0:
                    diffs = list(self.channel_diff_values[i])
                    x_diff = list(range(1, len(diffs) + 1))
                    self.channel_diff_lines[i].set_data(x_diff, diffs)
                    self.channel_diff_lines[i].set_visible(True)
                else:
                    self.channel_diff_lines[i].set_visible(False)
            else:
                self.channel_lines[i].set_visible(False)
                self.channel_diff_lines[i].set_visible(False)

        if has_data:
            self.ax.relim()
            self.ax.autoscale_view()
            self.canvas.draw_idle()

            active_channels = [i for i, cfg in enumerate(channel_configs) if cfg['active']]
            if active_channels:
                first = active_channels[0]
                if len(self.channel_values[first]) > 0:
                    last_val = self.channel_values[first][-1]
                    self.status_var.set(
                        f"{'Connected' if self.is_connected else 'Disconnected'} | "
                        f"Frames: {self.frame_count} | CH{first+1}: {last_val} | "
                        f"Active: {len(active_channels)} ch | [{self.view_mode_var.get()}]"
                    )

    def _redraw_plot(self):
        has_data = False
        for i in range(self.channel_count):
            visible = self.channel_visible[i] if i < len(self.channel_visible) else True
            if len(self.channel_values[i]) > 0 and visible:
                has_data = True
                vals = list(self.channel_values[i])
                x = list(range(len(vals)))
                self.channel_lines[i].set_data(x, vals)
                self.channel_lines[i].set_visible(True)
            else:
                self.channel_lines[i].set_visible(False)
            self.channel_diff_lines[i].set_visible(False)

        if has_data:
            self.ax.relim()
            self.ax.autoscale_view()
        self.canvas.draw_idle()

    def _clear_raw_view(self):
        self.raw_frame_buffer.clear()
        try:
            self.raw_text.config(state=tk.NORMAL)
            self.raw_text.delete("1.0", tk.END)
            self.raw_text.config(state=tk.DISABLED)
        except Exception:
            pass
        self._raw_update_pending = False
        self._raw_last_displayed_seq = None   # 修复2：seq 追踪一并复位

    def clear_plot(self):
        for i in range(self.channel_count):
            self.channel_values[i].clear()
            self.channel_diff_values[i].clear()
            self.channel_lines[i].set_data([], [])
            self.channel_diff_lines[i].set_data([], [])
        self.frame_count = 0
        self.sample_count = 0   # 修复5：样本计数一并清零
        self._extracted_save_buffer = []
        self._extracted_save_frame_start = 0
        self.ax.relim()
        self.canvas.draw()
        self._clear_raw_view()
        self.status_var.set("Cleared")

    def _on_mouse_wheel(self, event):
        if event.inaxes != self.ax:
            return
        zoom_factor = 1.1 if event.button == 'up' else 0.9

        xlim = self.ax.get_xlim()
        ylim = self.ax.get_ylim()

        mx = event.xdata
        my = event.ydata

        new_x_left = mx - (mx - xlim[0]) * zoom_factor
        new_x_right = mx + (xlim[1] - mx) * zoom_factor
        new_y_bottom = my - (my - ylim[0]) * zoom_factor
        new_y_top = my + (ylim[1] - my) * zoom_factor

        self.ax.set_xlim(new_x_left, new_x_right)
        self.ax.set_ylim(new_y_bottom, new_y_top)
        self.canvas.draw_idle()

    def load_file(self):
        filepath = filedialog.askopenfilename(
            title="Select Hex Data File",
            filetypes=[("Text files", "*.txt"), ("Hex files", "*.hex"), ("All files", "*.*")]
        )
        if not filepath:
            return
        try:
            with open(filepath, 'r') as f:
                content = f.read()
            self._analyze_hex_string(content)
        except Exception as e:
            messagebox.showerror("Error", str(e))

    def analyze_clipboard(self):
        try:
            content = self.root.clipboard_get()
            self._analyze_hex_string(content)
        except tk.TclError:
            messagebox.showwarning("Clipboard", "Clipboard is empty")
        except Exception as e:
            messagebox.showerror("Error", str(e))

    def _analyze_hex_string(self, hex_str):
        try:
            self.apply_frame_config()
        except:
            pass

        # 离线分析在主线程运行，可安全读取 UI
        channel_configs = self._read_channel_configs_from_ui()

        try:
            hex_str = re.sub(r'[\s\r\n]+', '', hex_str)
            data = bytes.fromhex(hex_str)
        except ValueError as e:
            messagebox.showerror("Parse Error", f"Invalid hex data: {e}")
            return

        header_len = len(self.frame_header)
        frames = []
        i = 0
        while i <= len(data) - self.frame_length:
            if data[i:i + header_len] == self.frame_header:
                frames.append(data[i:i + self.frame_length])
                i += self.frame_length
            else:
                i += 1

        if not frames:
            messagebox.showinfo("Result", "No valid frames found")
            return

        active_channels = [i for i, cfg in enumerate(channel_configs) if i < self.channel_count and cfg['active']]
        sample_size = 0
        if self.samples_per_packet > 1 and active_channels:
            all_indices = []
            for i in active_channels:
                all_indices.extend(channel_configs[i]['indices'])
            if all_indices:
                min_idx = min(all_indices)
                max_idx = max(all_indices)
                sample_size = max_idx - min_idx + 1

        channel_results = []
        for i in range(self.channel_count):
            channel_results.append([])

        num_samples = self.samples_per_packet if self.samples_per_packet > 1 else 1
        for frame in frames:
            for s in range(num_samples):
                for i in range(self.channel_count):
                    cfg = channel_configs[i] if i < len(channel_configs) else {'active': False}
                    if cfg['active']:
                        if self.samples_per_packet > 1:
                            adjusted_indices = [idx + s * sample_size for idx in cfg['indices']]
                        else:
                            adjusted_indices = cfg['indices']
                        val = self._extract_value_from_frame(frame, adjusted_indices, cfg['sign_bit'])
                        if val is not None:
                            channel_results[i].append(val)

        for i in range(self.channel_count):
            vals = channel_results[i]
            if vals:
                self.channel_values[i] = deque(vals, maxlen=max(self.max_points, len(vals)))
            else:
                self.channel_values[i].clear()
            self.channel_diff_values[i].clear()

        self.frame_count = len(frames)              # 修复5：帧级别计数
        self.sample_count = len(frames) * num_samples   # 修复5：样本级别计数

        # 修复8：离线分析后从 channel_values 全量重建差分队列
        self._rebuild_diff_values()

        for i in range(self.channel_count):
            cfg = channel_configs[i] if i < len(channel_configs) else {'active': False, 'diff': False}
            visible = self.channel_visible[i] if i < len(self.channel_visible) else True
            vals = list(self.channel_values[i])

            if cfg['active'] and vals and visible:
                x = list(range(len(vals)))
                self.channel_lines[i].set_data(x, vals)
                self.channel_lines[i].set_visible(True)

                # 修复8：差分曲线直接使用重建后的 channel_diff_values
                if cfg['diff'] and len(self.channel_diff_values[i]) > 0:
                    diffs = list(self.channel_diff_values[i])
                    x_diff = list(range(1, len(diffs) + 1))
                    self.channel_diff_lines[i].set_data(x_diff, diffs)
                    self.channel_diff_lines[i].set_visible(True)
                else:
                    self.channel_diff_lines[i].set_visible(False)
            else:
                self.channel_lines[i].set_visible(False)
                self.channel_diff_lines[i].set_visible(False)

        self.ax.relim()
        self.ax.autoscale_view()
        self.canvas.draw()

        active_count = sum(1 for cfg in channel_configs if cfg['active'])
        msg = f"Found {len(frames)} frames\n"
        msg += f"Active channels: {active_count}\n\n"

        for i in range(self.channel_count):
            cfg = channel_configs[i] if i < len(channel_configs) else {'active': False}
            if cfg['active'] and channel_results[i]:
                vals = channel_results[i]
                msg += f"CH{i+1}: {len(vals)} values\n"
                msg += f"  Range: {min(vals)} ~ {max(vals)}\n"
                msg += f"  First: {vals[0]} (0x{vals[0] & 0xFFFF:04X})\n"
                msg += f"  Last: {vals[-1]} (0x{vals[-1] & 0xFFFF:04X})\n\n"

        messagebox.showinfo("Analysis Complete", msg)


if __name__ == "__main__":
    # 2026-11-03 修复B①：高DPI感知必须在创建 Tk root 之前调用
    setup_windows_dpi_awareness()
    root = tk.Tk()
    app = FrameAnalyzer(root)
    root.mainloop()
