#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Binary Frame Analyzer - 解析串口二进制帧数据并绘图
支持按帧头同步、指定字节索引提取数值、实时/离线模式
支持多路数据同时绘图、每路独立符号位配置、一阶差分
"""
import re
import sys
import tkinter as tk
from tkinter import ttk, scrolledtext, messagebox, filedialog
import serial
import serial.tools.list_ports
import socket
import threading
import time
from matplotlib.backends.backend_tkagg import FigureCanvasTkAgg, NavigationToolbar2Tk
from matplotlib.figure import Figure
from collections import deque

# 通道颜色（matplotlib 默认配色方案）
CHANNEL_COLORS = [
    '#1f77b4',  # 蓝
    '#ff7f0e',  # 橙
    '#2ca02c',  # 绿
    '#d62728',  # 红
    '#9467bd',  # 紫
    '#8c564b',  # 棕
    '#e377c2',  # 粉
    '#7f7f7f',  # 灰
]
MAX_CHANNELS = 8


class FrameAnalyzer:
    def __init__(self, root):
        self.root = root
         # ========== 新增：设置窗口ICO图标 ==========
        import os
        ico_path = "app.ico"
        if not os.path.exists(ico_path):
            ico_path = "./app.ico"
        if os.path.exists(ico_path):
            self.root.iconbitmap(ico_path)
    # ==========================================
        self.root.title("Binary Frame Analyzer by gykjwrq@163.com ")
        self.root.geometry("1100x900")
        self.root.minsize(900, 700)  # 最小窗口尺寸，防止布局混乱

        # Serial
        self.serial_port = None
        self.is_connected = False
        self.receive_thread = None
        self.stop_event = threading.Event()
        self.rx_buffer = bytearray()

        # Frame config
        self.frame_header = bytes.fromhex("EB90")
        self.frame_length = 11  # bytes per frame
        self.byte_order = "big"  # big / little
        self.samples_per_packet = 1

        # UDP settings
        self.interface_type = "serial"  # "serial" or "udp"
        self.udp_local_port = 8080
        self.udp_remote_ip = ""
        self.udp_remote_port = 0
        self.udp_socket = None
        self.udp_remote_addr = None

        # Plot config
        self.max_points = 500

        # Channel config
        self.channel_count = 1
        self.channels = []  # list of dict: {'indices': [...], 'sign_bit': int, 'diff': bool}
        self.channel_values = []  # list of deque
        self.channel_diff_values = []  # list of deque (一阶差分)
        self.channel_lines = []  # list of Line2D
        self.channel_diff_lines = []  # list of Line2D
        self.frame_count = 0

        # Channel widget refs (for dynamic creation)
        self.channel_widgets = []  # list of dict: {'indices_var', 'sign_bit_var', 'diff_var', 'frame'}
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

        # Initialize default channel
        self._init_channels(1)

        self.create_widgets()
        self.refresh_ports()
        self._on_interface_change()  # Initialize interface visibility

    def _init_channels(self, count):
        """初始化通道数据结构"""
        self.channel_count = count
        self.channels = []
        self.channel_values = []
        self.channel_diff_values = []
        self.channel_visible = []

        for i in range(count):
            # 默认配置：第1路用2,3，后面的依次往后推
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

    def create_widgets(self):
        main_frame = ttk.Frame(self.root, padding="5")
        main_frame.grid(row=0, column=0, sticky=(tk.W, tk.E, tk.N, tk.S))
        self.root.columnconfigure(0, weight=1)
        self.root.rowconfigure(0, weight=1)

        # 左右分栏布局
        main_frame.columnconfigure(0, weight=0)   # 左侧设置区固定宽度
        main_frame.columnconfigure(1, weight=1)   # 右侧绘图区占据剩余空间
        main_frame.rowconfigure(0, weight=1)

        # ========== 左侧设置区 ==========
        left_frame = ttk.Frame(main_frame, width=340)
        left_frame.grid(row=0, column=0, sticky=(tk.W, tk.E, tk.N, tk.S), padx=(0, 5))
        left_frame.grid_propagate(False)
        left_frame.columnconfigure(0, weight=1)

        # --- Connection ---
        port_frame = ttk.LabelFrame(left_frame, text="Connection", padding="8")
        port_frame.grid(row=0, column=0, sticky=(tk.W, tk.E), pady=3)
        port_frame.columnconfigure(1, weight=1)
        port_frame.columnconfigure(3, weight=1)

        ttk.Label(port_frame, text="Interface:").grid(row=0, column=0, padx=3, pady=2, sticky=tk.W)
        self.interface_var = tk.StringVar(value="Serial")
        interface_combo = ttk.Combobox(port_frame, textvariable=self.interface_var,
                                       values=["Serial", "UDP"], width=7, state="readonly")
        interface_combo.grid(row=0, column=1, padx=3, pady=2, sticky=tk.W)
        interface_combo.bind("<<ComboboxSelected>>", self._on_interface_change)
        ttk.Button(port_frame, text="Help", command=self.show_help, width=6).grid(row=0, column=2, padx=3, pady=2, sticky=tk.E)

        # --- Separator ---
        ttk.Separator(port_frame, orient=tk.HORIZONTAL).grid(row=1, column=0, columnspan=4, padx=3, pady=5, sticky=(tk.W, tk.E))

        # --- Serial settings frame ---
        self.serial_frame = ttk.Frame(port_frame)
        self.serial_frame.grid(row=2, column=0, columnspan=3, sticky=(tk.W, tk.E))
        self.serial_frame.columnconfigure(1, weight=1)

        ttk.Label(self.serial_frame, text="Port:").grid(row=0, column=0, padx=3, pady=2, sticky=tk.W)
        self.port_var = tk.StringVar()
        self.port_combo = ttk.Combobox(self.serial_frame, textvariable=self.port_var, width=12)
        self.port_combo.grid(row=0, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))
        ttk.Button(self.serial_frame, text="Refresh", command=self.refresh_ports, width=8).grid(row=0, column=2, padx=3, pady=2)

        ttk.Label(self.serial_frame, text="Baud:").grid(row=1, column=0, padx=3, pady=2, sticky=tk.W)
        self.baud_var = tk.StringVar(value="1000000")
        baud_combo = ttk.Combobox(self.serial_frame, textvariable=self.baud_var,
                     values=["9600", "19200", "38400", "57600", "115200", "230400", "460800", "921600",
                             "1000000", "1500000", "2000000", "2500000", "3000000"],
                     width=12)
        baud_combo.grid(row=1, column=1, columnspan=2, padx=3, pady=2, sticky=(tk.W, tk.E))

        # --- UDP settings frame ---
        self.udp_frame = ttk.Frame(port_frame)
        self.udp_frame.grid(row=2, column=0, columnspan=3, sticky=(tk.W, tk.E))
        self.udp_frame.columnconfigure(1, weight=1)

        ttk.Label(self.udp_frame, text="Local IP:").grid(row=0, column=0, padx=3, pady=2, sticky=tk.W)
        self.udp_local_ip_var = tk.StringVar(value="0.0.0.0")
        ttk.Entry(self.udp_frame, textvariable=self.udp_local_ip_var, width=12).grid(row=0, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        ttk.Label(self.udp_frame, text="Local Port:").grid(row=1, column=0, padx=3, pady=2, sticky=tk.W)
        self.udp_local_port_var = tk.StringVar(value="8080")
        ttk.Entry(self.udp_frame, textvariable=self.udp_local_port_var, width=12).grid(row=1, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        ttk.Label(self.udp_frame, text="Remote IP:").grid(row=2, column=0, padx=3, pady=2, sticky=tk.W)
        self.udp_remote_ip_var = tk.StringVar(value="")
        ttk.Entry(self.udp_frame, textvariable=self.udp_remote_ip_var, width=12).grid(row=2, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        ttk.Label(self.udp_frame, text="Remote Port:").grid(row=3, column=0, padx=3, pady=2, sticky=tk.W)
        self.udp_remote_port_var = tk.StringVar(value="")
        ttk.Entry(self.udp_frame, textvariable=self.udp_remote_port_var, width=12).grid(row=3, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        # --- Connect button ---
        self.connect_btn = ttk.Button(port_frame, text="Connect", command=self.toggle_connection)
        self.connect_btn.grid(row=3, column=0, columnspan=3, padx=3, pady=5, sticky=(tk.W, tk.E))

        ttk.Separator(port_frame, orient=tk.HORIZONTAL).grid(row=4, column=0, columnspan=4, padx=3, pady=5, sticky=(tk.W, tk.E))

        ttk.Button(port_frame, text="Load File", command=self.load_file).grid(row=5, column=0, padx=3, pady=2, sticky=(tk.W, tk.E))
        ttk.Button(port_frame, text="Analyze Clipboard", command=self.analyze_clipboard).grid(row=5, column=1, columnspan=2, padx=3, pady=2, sticky=(tk.W, tk.E))

        # Auto Save (Raw Data)
        self.auto_save_var = tk.BooleanVar(value=False)
        ttk.Checkbutton(port_frame, text="Auto Save", variable=self.auto_save_var,
                        command=self.toggle_auto_save).grid(row=6, column=0, padx=3, pady=5, sticky=tk.W)
        ttk.Button(port_frame, text="Save File...", command=self.select_save_file, width=10).grid(row=6, column=1, padx=3, pady=5)
        self.save_file_label = ttk.Label(port_frame, text="(no file)", foreground="gray", wraplength=200)
        self.save_file_label.grid(row=7, column=0, columnspan=3, padx=3, pady=1, sticky=tk.W)

        # Extracted Data Save
        self.extracted_save_var = tk.BooleanVar(value=False)
        ttk.Checkbutton(port_frame, text="Save Extracted", variable=self.extracted_save_var,
                        command=self.toggle_extracted_save).grid(row=8, column=0, padx=3, pady=2, sticky=tk.W)
        ttk.Button(port_frame, text="CSV File...", command=self.select_extracted_save_file, width=10).grid(row=8, column=1, padx=3, pady=2)
        self.extracted_save_label = ttk.Label(port_frame, text="(no file)", foreground="gray", wraplength=200)
        self.extracted_save_label.grid(row=9, column=0, columnspan=3, padx=3, pady=1, sticky=tk.W)

        # --- Frame Configuration ---
        frame_cfg = ttk.LabelFrame(left_frame, text="Frame Configuration", padding="8")
        frame_cfg.grid(row=1, column=0, sticky=(tk.W, tk.E), pady=3)
        frame_cfg.columnconfigure(1, weight=1)

        ttk.Label(frame_cfg, text="Frame Header:").grid(row=0, column=0, padx=3, pady=2, sticky=tk.W)
        self.header_var = tk.StringVar(value="EB 90")
        ttk.Entry(frame_cfg, textvariable=self.header_var, width=12).grid(row=0, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        ttk.Label(frame_cfg, text="Frame Length:").grid(row=1, column=0, padx=3, pady=2, sticky=tk.W)
        self.length_var = tk.StringVar(value="11")
        ttk.Entry(frame_cfg, textvariable=self.length_var, width=12).grid(row=1, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        ttk.Label(frame_cfg, text="Byte Order:").grid(row=2, column=0, padx=3, pady=2, sticky=tk.W)
        self.order_var = tk.StringVar(value="big")
        ttk.Combobox(frame_cfg, textvariable=self.order_var, values=["big", "little"], width=10).grid(row=2, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        ttk.Label(frame_cfg, text="Samples/Pkt:").grid(row=3, column=0, padx=3, pady=2, sticky=tk.W)
        self.samples_var = tk.StringVar(value="1")
        ttk.Entry(frame_cfg, textvariable=self.samples_var, width=12).grid(row=3, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        ttk.Label(frame_cfg, text="Max Points:").grid(row=4, column=0, padx=3, pady=2, sticky=tk.W)
        self.max_pts_var = tk.StringVar(value="500")
        ttk.Entry(frame_cfg, textvariable=self.max_pts_var, width=12).grid(row=4, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        ttk.Button(frame_cfg, text="Apply Frame", command=self.apply_frame_config).grid(row=5, column=0, padx=3, pady=5, sticky=(tk.W, tk.E))
        ttk.Button(frame_cfg, text="Clear Plot", command=self.clear_plot).grid(row=5, column=1, padx=3, pady=5, sticky=(tk.W, tk.E))

        # --- Channel Configuration ---
        chan_frame = ttk.LabelFrame(left_frame, text="Channel Configuration", padding="8")
        chan_frame.grid(row=2, column=0, sticky=(tk.W, tk.E, tk.N, tk.S), pady=3)
        chan_frame.columnconfigure(0, weight=1)
        chan_frame.rowconfigure(2, weight=1)
        left_frame.rowconfigure(2, weight=1)

        # Channel count
        ttk.Label(chan_frame, text="Channels:").grid(row=0, column=0, padx=3, pady=2, sticky=tk.W)
        self.channel_count_var = tk.StringVar(value="1")
        chan_count_entry = ttk.Entry(chan_frame, textvariable=self.channel_count_var, width=5)
        chan_count_entry.grid(row=0, column=1, padx=3, pady=2, sticky=tk.W)
        ttk.Button(chan_frame, text="Apply", command=self.apply_channel_config).grid(row=0, column=2, padx=3, pady=2)

        # Color legend
        self.color_legend_frame = ttk.Frame(chan_frame)
        self.color_legend_frame.grid(row=1, column=0, columnspan=3, sticky=(tk.W, tk.E), pady=3)

        # Channels container (scrollable)
        self.channels_canvas = tk.Canvas(chan_frame, highlightthickness=0)
        self.channels_scroll = ttk.Scrollbar(chan_frame, orient="vertical", command=self.channels_canvas.yview)
        self.channels_container = ttk.Frame(self.channels_canvas)

        self.channels_container.bind(
            "<Configure>",
            lambda e: self.channels_canvas.configure(scrollregion=self.channels_canvas.bbox("all"))
        )
        self.channels_canvas.create_window((0, 0), window=self.channels_container, anchor="nw")
        self.channels_canvas.configure(yscrollcommand=self.channels_scroll.set)

        self.channels_canvas.grid(row=2, column=0, columnspan=2, sticky=(tk.W, tk.E, tk.N, tk.S), pady=3)
        self.channels_scroll.grid(row=2, column=2, sticky=(tk.N, tk.S), pady=3)

        # Create initial channel widgets
        self._create_channel_widgets()
        self._update_color_legend()

        # ========== 右侧主区域 ==========
        right_frame = ttk.Frame(main_frame)
        right_frame.grid(row=0, column=1, sticky=(tk.W, tk.E, tk.N, tk.S))
        right_frame.columnconfigure(0, weight=1)
        right_frame.rowconfigure(0, weight=1)  # 绘图区占据大部分空间

        # --- Plot Area ---
        plot_frame = ttk.LabelFrame(right_frame, text="Data Plot", padding="5")
        plot_frame.grid(row=0, column=0, sticky=(tk.W, tk.E, tk.N, tk.S))
        plot_frame.rowconfigure(0, weight=1)
        plot_frame.columnconfigure(0, weight=1)

        self.fig = Figure(figsize=(8, 5), dpi=100)
        self.ax = self.fig.add_subplot(111)
        self.ax.set_xlabel('Frame Number')
        self.ax.set_ylabel('Value')
        self.ax.set_title('Extracted Data vs Frame Index')
        self.ax.grid(True, alpha=0.3)

        self.canvas = FigureCanvasTkAgg(self.fig, master=plot_frame)
        self.canvas.draw()
        self.canvas.get_tk_widget().grid(row=0, column=0, sticky=(tk.W, tk.E, tk.N, tk.S))

        # Navigation toolbar
        toolbar_frame = ttk.Frame(plot_frame)
        toolbar_frame.grid(row=1, column=0, sticky=(tk.W, tk.E))
        self.toolbar = NavigationToolbar2Tk(self.canvas, toolbar_frame)
        self.toolbar.update()

        # Mouse wheel zoom
        self.canvas.mpl_connect('scroll_event', self._on_mouse_wheel)

        # Initialize plot lines
        self._init_plot_lines()

        # --- Send ---
        tx_frame = ttk.LabelFrame(right_frame, text="Send", padding="5")
        tx_frame.grid(row=1, column=0, sticky=(tk.W, tk.E), pady=(5, 0))
        tx_frame.columnconfigure(1, weight=1)

        ttk.Label(tx_frame, text="Data:").grid(row=0, column=0, padx=3, pady=2, sticky=tk.W)
        self.tx_data_var = tk.StringVar(value="EB 90 01 02 03 04 05 06 07 08 00")
        self.tx_entry = ttk.Entry(tx_frame, textvariable=self.tx_data_var)
        self.tx_entry.grid(row=0, column=1, padx=3, pady=2, sticky=(tk.W, tk.E))

        self.tx_format_var = tk.StringVar(value="hex")
        ttk.Radiobutton(tx_frame, text="HEX", variable=self.tx_format_var, value="hex").grid(row=0, column=2, padx=3, pady=2)
        ttk.Radiobutton(tx_frame, text="ASCII", variable=self.tx_format_var, value="ascii").grid(row=0, column=3, padx=3, pady=2)

        ttk.Button(tx_frame, text="Send", command=self.send_data, width=10).grid(row=0, column=4, padx=5, pady=2)
        ttk.Button(tx_frame, text="History", command=self.show_send_history, width=10).grid(row=0, column=5, padx=3, pady=2)

        self.tx_count = 0
        self.send_history = []  # List of (format, data_str)
        self.max_history = 50
        self.tx_count_var = tk.StringVar(value="TX: 0")
        ttk.Label(tx_frame, textvariable=self.tx_count_var).grid(row=0, column=6, padx=10, pady=2)

        # --- Status Bar ---
        info_frame = ttk.Frame(right_frame)
        info_frame.grid(row=2, column=0, sticky=(tk.W, tk.E), pady=(5, 0))

        self.status_var = tk.StringVar(value="Disconnected | Frames: 0 | Channels: 1")
        ttk.Label(info_frame, textvariable=self.status_var, relief=tk.SUNKEN).pack(fill=tk.X)

    def _create_channel_widgets(self):
        """动态创建通道配置控件（单列紧凑布局）"""
        # Clear existing
        for w in self.channels_container.winfo_children():
            w.destroy()
        self.channel_widgets = []

        for i in range(self.channel_count):
            color = CHANNEL_COLORS[i % len(CHANNEL_COLORS)]
            chan_data = self.channels[i] if i < len(self.channels) else {'indices': [2, 3], 'sign_bit': -1, 'diff': False}

            # Each channel group has its own sub-frame (single column)
            chan_frame = ttk.Frame(self.channels_container)
            chan_frame.grid(row=i, column=0, padx=5, pady=4, sticky=(tk.W, tk.E))
            chan_frame.columnconfigure(1, weight=1)
            chan_frame.columnconfigure(3, weight=1)

            # Row 0: Colored checkbox (with CH text) + Idx + Sign
            visible = self.channel_visible[i] if i < len(self.channel_visible) else True
            visible_var = tk.BooleanVar(value=visible)
            visible_check = tk.Checkbutton(
                chan_frame,
                text=f"CH{i+1}",
                variable=visible_var,
                bg=color,
                fg='white',
                selectcolor=color,
                activebackground=color,
                activeforeground='white',
                font=('Arial', 9, 'bold'),
                command=lambda idx=i, v=visible_var: self._toggle_channel_visibility(idx, v)
            )
            visible_check.grid(row=0, column=0, padx=2, pady=2, sticky=tk.W)

            ttk.Label(chan_frame, text="Idx:", font=('Arial', 8)).grid(row=0, column=1, padx=(10, 2), pady=2, sticky=tk.E)
            indices_str = ','.join(str(x) for x in chan_data['indices'])
            indices_var = tk.StringVar(value=indices_str)
            indices_entry = ttk.Entry(chan_frame, textvariable=indices_var, width=8)
            indices_entry.grid(row=0, column=2, padx=2, pady=2, sticky=(tk.W, tk.E))

            ttk.Label(chan_frame, text="Sign:", font=('Arial', 8)).grid(row=0, column=3, padx=(10, 2), pady=2, sticky=tk.E)
            sign_bit_var = tk.StringVar(value=str(chan_data['sign_bit']))
            sign_bit_entry = ttk.Entry(chan_frame, textvariable=sign_bit_var, width=4)
            sign_bit_entry.grid(row=0, column=4, padx=2, pady=2, sticky=tk.W)

            # Bind auto-update sign bit when indices change
            indices_entry.bind('<FocusOut>', lambda e, sv=sign_bit_var, iv=indices_var: self._auto_update_sign_bit(sv, iv))
            indices_entry.bind('<Return>', lambda e, sv=sign_bit_var, iv=indices_var: self._auto_update_sign_bit(sv, iv))

            # Row 1: 1st Diff checkbox
            diff_var = tk.BooleanVar(value=chan_data['diff'])
            diff_check = ttk.Checkbutton(chan_frame, text="1st Diff", variable=diff_var)
            diff_check.grid(row=1, column=0, columnspan=2, padx=2, pady=1, sticky=tk.W)

            self.channel_widgets.append({
                'indices_var': indices_var,
                'sign_bit_var': sign_bit_var,
                'diff_var': diff_var,
                'visible_var': visible_var,
            })

    def _update_color_legend(self):
        """更新颜色图例"""
        for w in self.color_legend_frame.winfo_children():
            w.destroy()

        for i in range(min(self.channel_count, len(CHANNEL_COLORS))):
            color = CHANNEL_COLORS[i]
            tk.Label(self.color_legend_frame, text=f" CH{i+1} ", bg=color, fg='white',
                     font=('Arial', 8, 'bold')).pack(side=tk.LEFT, padx=2)

    def _toggle_channel_visibility(self, channel_idx, var):
        """切换通道显示/隐藏"""
        if channel_idx < len(self.channel_visible):
            self.channel_visible[channel_idx] = var.get()
            self._redraw_plot()

    def _auto_update_sign_bit(self, sign_bit_var, indices_var):
        """根据idx字节数自动更新sign bit默认值"""
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
        """初始化绘图曲线"""
        # Clear existing lines
        for line in self.channel_lines:
            line.remove()
        for line in self.channel_diff_lines:
            line.remove()
        self.channel_lines = []
        self.channel_diff_lines = []

        for i in range(self.channel_count):
            color = CHANNEL_COLORS[i % len(CHANNEL_COLORS)]
            # Original data line (solid)
            line, = self.ax.plot([], [], color=color, linewidth=1.2, marker='.', markersize=3,
                                 label=f'CH{i+1}')
            self.channel_lines.append(line)

            # Diff line (dashed, same color)
            diff_line, = self.ax.plot([], [], color=color, linewidth=1, linestyle='--', alpha=0.7,
                                      label=f'CH{i+1} diff')
            self.channel_diff_lines.append(diff_line)
            # Initially hide diff lines
            diff_line.set_visible(False)

        # Update legend
        self.ax.legend(loc='upper left', fontsize=8)

    def refresh_ports(self):
        ports = serial.tools.list_ports.comports()
        port_list = [p.device for p in ports]
        self.port_combo['values'] = port_list
        if port_list:
            self.port_combo.current(0)

    def apply_frame_config(self):
        """应用帧配置"""
        try:
            header_str = self.header_var.get().replace(' ', '').replace('\n', '')
            self.frame_header = bytes.fromhex(header_str)
            self.frame_length = int(self.length_var.get())
            self.byte_order = self.order_var.get()
            self.samples_per_packet = int(self.samples_var.get())
            if self.samples_per_packet < 1:
                self.samples_per_packet = 1
            self.max_points = int(self.max_pts_var.get())

            # Resize all channel buffers
            for i in range(len(self.channel_values)):
                self.channel_values[i] = deque(self.channel_values[i], maxlen=self.max_points)
                self.channel_diff_values[i] = deque(self.channel_diff_values[i], maxlen=self.max_points)

            messagebox.showinfo("Config", "Frame configuration applied")
        except Exception as e:
            messagebox.showerror("Config Error", str(e))

    def _on_interface_change(self, event=None):
        """Interface切换回调：显示对应设置，隐藏另一个"""
        # Auto disconnect if connected
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
        """应用通道配置"""
        try:
            count = int(self.channel_count_var.get())
            if count < 1:
                count = 1
            if count > MAX_CHANNELS:
                count = MAX_CHANNELS
                self.channel_count_var.set(str(count))

            # Read current values from widgets before recreating
            old_count = len(self.channel_widgets)
            old_configs = []
            for i in range(old_count):
                w = self.channel_widgets[i]
                indices_str = w['indices_var'].get()
                indices = self._parse_indices(indices_str) if indices_str.strip() else []
                sign_bit = int(w['sign_bit_var'].get()) if w['sign_bit_var'].get().strip() else -1
                diff = w['diff_var'].get()
                old_configs.append({'indices': indices, 'sign_bit': sign_bit, 'diff': diff})

            # Update channel count and data
            self.channel_count = count

            # Preserve existing configs where possible
            new_channels = []
            new_values = []
            new_diff_values = []
            new_visible = []

            for i in range(count):
                if i < len(old_configs) and old_configs[i]['indices']:
                    cfg = old_configs[i]
                else:
                    # Default config
                    start_idx = 2 + i * 2
                    cfg = {'indices': [start_idx, start_idx + 1], 'sign_bit': -1, 'diff': False}
                new_channels.append(cfg)
                new_values.append(deque(maxlen=self.max_points))
                new_diff_values.append(deque(maxlen=self.max_points))
                # Preserve visibility setting, default True for new channels
                if i < len(self.channel_visible):
                    new_visible.append(self.channel_visible[i])
                else:
                    new_visible.append(True)

            self.channels = new_channels
            self.channel_values = new_values
            self.channel_diff_values = new_diff_values
            self.channel_visible = new_visible

            # Recreate widgets
            self._create_channel_widgets()
            self._update_color_legend()

            # Recreate plot lines
            self._init_plot_lines()

            # Redraw
            self._redraw_plot()

            messagebox.showinfo("Config", f"Channel configuration applied ({count} channels)")
        except Exception as e:
            messagebox.showerror("Config Error", str(e))

    def _parse_indices(self, indices_str):
        """解析索引字符串，支持多种分隔符"""
        return [int(x.strip()) for x in re.split(r'[,\s，、]+', indices_str) if x.strip()]

    def _read_channel_configs_from_ui(self):
        """从界面读取当前通道配置"""
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

    def sign_extend(self, value, sign_bit_pos):
        """符号扩展"""
        if sign_bit_pos < 0:
            return value
        sign_mask = 1 << sign_bit_pos
        if value & sign_mask:
            extend_mask = ~((1 << (sign_bit_pos + 1)) - 1)
            value = value | extend_mask
        return value

    def _extract_value_from_frame(self, frame, indices, sign_bit):
        """从单帧中提取单个通道的值"""
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

    def _compute_first_diff(self, values_deque):
        """计算一阶差分"""
        if len(values_deque) < 2:
            return []
        vals = list(values_deque)
        diffs = []
        for i in range(1, len(vals)):
            diffs.append(vals[i] - vals[i - 1])
        return diffs

    def toggle_connection(self):
        if self.is_connected:
            self.disconnect()
        else:
            self.connect()

    def connect(self):
        try:
            # Apply current configs before connecting
            self.apply_frame_config()
            self._read_channel_configs_from_ui()

            self.stop_event.clear()
            self.rx_buffer.clear()
            self.frame_count = 0

            # Clear all channel buffers
            for i in range(self.channel_count):
                self.channel_values[i].clear()
                self.channel_diff_values[i].clear()

            # Reset extracted save buffer
            self._extracted_save_buffer = []
            self._extracted_save_frame_start = 0

            if self.interface_type == "serial":
                port = self.port_var.get()
                if not port:
                    messagebox.showerror("Error", "Select a port first")
                    return

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
                self.connect_btn.config(text="Disconnect")
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

                # Store remote address if specified
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
                self.connect_btn.config(text="Disconnect")
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
        self.connect_btn.config(text="Connect")
        self.status_var.set("Disconnected")

        # Flush remaining extracted data
        if self._extracted_save_buffer and self.extracted_save_file:
            try:
                with open(self.extracted_save_file, 'a') as f:
                    # Write header if not written yet
                    if not self._extracted_header_written:
                        channel_configs = self._read_channel_configs_from_ui()
                        active_channels = [i for i, cfg in enumerate(channel_configs) if cfg['active']]
                        header = "Frame," + ",".join(f"CH{i+1}" for i in active_channels) + "\n"
                        f.write(header)
                        self._extracted_header_written = True

                    for frame_num, values in self._extracted_save_buffer:
                        line = f"{frame_num}," + ",".join(str(v) for v in values) + "\n"
                        f.write(line)
                self._extracted_save_buffer = []
            except Exception as e:
                print(f"Extracted save flush error: {e}")

    def send_data(self):
        """Send data (serial/UDP)"""
        if not self.is_connected:
            messagebox.showerror("Error", "Not connected")
            return

        # Parse data
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
            # Add to history (avoid duplicates of last entry)
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


    def show_send_history(self):
        """Show send history dialog"""
        if not self.send_history:
            messagebox.showinfo("History", "No send history yet.")
            return

        dlg = tk.Toplevel(self.root)
        dlg.title("Send History")
        dlg.geometry("500x400")
        dlg.transient(self.root)
        dlg.grab_set()

        # Listbox with scrollbar
        frame = ttk.Frame(dlg, padding="10")
        frame.pack(fill=tk.BOTH, expand=True)

        ttk.Label(frame, text=f"Last {len(self.send_history)} commands (newest at bottom):").pack(anchor=tk.W)

        list_frame = ttk.Frame(frame)
        list_frame.pack(fill=tk.BOTH, expand=True, pady=5)

        scrollbar = ttk.Scrollbar(list_frame)
        scrollbar.pack(side=tk.RIGHT, fill=tk.Y)

        listbox = tk.Listbox(list_frame, yscrollcommand=scrollbar.set, font=("Consolas", 10))
        listbox.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        scrollbar.config(command=listbox.yview)

        # Populate listbox (newest first)
        for i, (fmt, data) in enumerate(reversed(self.send_history)):
            preview = data[:60].replace('\n', ' ')
            if len(data) > 60:
                preview += "..."
            listbox.insert(tk.END, f"[{fmt.upper()}] {preview}")

        def on_select(event=None):
            selection = listbox.curselection()
            if not selection:
                return
            # Convert from reversed index to actual index
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

        # Buttons
        btn_frame = ttk.Frame(frame)
        btn_frame.pack(fill=tk.X, pady=5)

        ttk.Button(btn_frame, text="Load to Input", command=on_select).pack(side=tk.LEFT, padx=5)
        ttk.Button(btn_frame, text="Send Immediately", command=on_send).pack(side=tk.LEFT, padx=5)
        ttk.Button(btn_frame, text="Cancel", command=dlg.destroy).pack(side=tk.RIGHT, padx=5)

        # Select first item by default
        if self.send_history:
            listbox.selection_set(0)
            listbox.activate(0)

    def show_help(self):
        """显示帮助文档"""
        dlg = tk.Toplevel(self.root)
        dlg.title("帮助 - 二进制帧分析工具")
        dlg.geometry("700x600")
        dlg.transient(self.root)

        # Create text widget with scrollbar
        frame = ttk.Frame(dlg, padding="10")
        frame.pack(fill=tk.BOTH, expand=True)

        text = scrolledtext.ScrolledText(frame, wrap=tk.WORD, font=("TkDefaultFont", 10))
        text.pack(fill=tk.BOTH, expand=True)

        help_text = """
╔═════════════════════════════════════╗
║  二进制帧分析工具(帧析助手) 使用说明                                  ║
║  作者: RhycWu                                                       ║
║  邮箱: gykjwrq@163.com                                              ║
╚═════════════════════════════════════╝

【首次使用】

 python版
  1. 安装 Python 3.8 或更高版本
  2. 安装依赖库：
     pip install pyserial matplotlib

  3. 运行程序：
     python 8.py

 Windows版
  双击exe

【接口选择】

  串口（Serial）：
    - 端口（Port）：选择串口号（点击 Refresh 刷新列表）
    - 波特率（Baud）：9600 ~ 3000000 可选

  网口（UDP）：
    - 本地 IP（Local IP）：绑定的网卡地址（默认 0.0.0.0 = 所有网卡）
    - 本地端口（Local Port）：监听的 UDP 端口
    - 远程 IP（Remote IP）：发送数据的目标 IP（可选）
    - 远程端口（Remote Port）：发送数据的目标端口（可选）
    - 说明：如果不填远程 IP/端口，程序会自动回复给向你发送数据的地址


【帧配置（Frame Configuration）】

  帧头（Frame Header）：
    - 标记一帧开始的十六进制字节
    - 例如：EB 90
    - 可以是任意长度（1字节、2字节等）

  帧长（Frame Length）：
    - 每帧的总字节数（包含帧头）
    - 多采样点大包模式下，填整个数据包的大小

  字节序（Byte Order）：
    - 大端（Big Endian）：高位字节在前
    - 小端（Little Endian）：低位字节在前

  每包采样数（Samples/Pkt）：
    - 每个数据包中包含的采样点数量
    - 1 = 普通模式（每帧一个采样点，和串口一样）
    - >1 = 大包模式（每帧多个采样点）
    - 程序会根据通道索引配置自动计算每个采样点的大小

  最大显示点数（Max Points）：
    - 曲线图上最多显示多少个数据点
    - 超过后旧数据从左侧滚动消失

  应用帧配置（Apply Frame）：使帧配置生效
  清空曲线（Clear Plot）：清空所有绘图数据


【通道配置（Channel Configuration）】

  通道数量：1 ~ 8 路

  每个通道包含：
    - 索引（Idx）：通道数据在帧中的字节位置（从0开始）
      * 用逗号分隔：3,4 （2字节）
      * 支持分隔符：英文逗号、空格、中文逗号、中文顿号
      * 例如：通道数据在第5和第6字节，输入：5,6

    - 符号位（Sign）：符号位的位置（-1 表示无符号）
      * 例如：16位有符号数，填 15（第15位是符号位）
      * 修改索引后会自动更新默认值

    - 一阶差分（1st Diff）：启用一阶差分计算
      * 显示相邻两个采样点的差值
      * 用虚线绘制

  应用通道配置（Apply Channel）：使通道配置生效


【数据发送】

  发送格式：
    - HEX：发送原始十六进制字节（空格/换行分隔）
    - ASCII：发送文本，按ASCII编码

  历史记录（History）按钮：
    - 查看之前发送过的命令
    - 双击或点"立即发送"可直接重发
    - 点"载入输入框"可加载到发送框修改后再发


【数据保存】

  自动保存原始数据（Auto Save）：
    - 自动将所有接收到的原始数据保存到文件
    - 格式：空格分隔的十六进制字节
    - 点击"Save File..."选择保存文件

  保存提取数据（Save Extracted）：
    - 自动将提取后的通道数值保存为CSV文件
    - 格式：帧号, 通道1, 通道2, 通道3, ...
    - 点击"CSV File..."选择保存文件


【离线分析】

  加载文件（Load File）：
    - 加载十六进制数据文件进行离线分析
    - 显示内容：帧数、各通道值范围、首尾值

  分析剪贴板（Analyze Clipboard）：
    - 分析复制到剪贴板的十六进制数据
    - 输出格式和加载文件相同


【多采样点大包模式（UDP）】

  当 Samples/Pkt > 1 时：
    - 每个 UDP 数据包包含多个采样点
    - 程序会自动提取每个包中的所有采样点
    - 所有采样点正常绘图和保存
    - 包尾部多余的字节自动忽略

  示例：832字节包，100个采样点
    - 帧头：EB 90
    - 帧长：832
    - 每包采样数：100
    - CH1 索引：3,4 （第一个采样点的通道1在第3、4字节）
    - CH2 索引：5,6
    - ...
    - 程序自动计算：每个采样点 = 8 字节
    - 100个采样点 × 8字节 = 800字节采样数据
    - 剩余字节（832 - 2 - 800 = 30）自动忽略


【鼠标和键盘操作】

  曲线图：
    - 滚轮：放大/缩小
    - 左键拖动：平移
    - 右键菜单：更多功能（缩放、保存等）

  发送：
    - 在发送框中按回车直接发送


【常见问题】

  "无法打开串口"：
    1. 点击 Refresh 刷新，确认端口是否存在
    2. 确认没有其他程序占用该串口
    3. 检查 USB 线是否连接好

  "收不到数据"：
    1. 确认波特率/端口设置正确
    2. 确认帧头和数据格式匹配
    3. 检查字节序（大端/小端）是否正确

  "UDP 无法绑定端口"：
    1. 端口可能被其他程序占用
    2. 换一个端口号试试
    3. 如果端口 < 1024，需要管理员权限

  "没有远程地址"（UDP发送）：
    1. 手动填写远程 IP 和远程端口
    2. 或者先接收一次数据（程序会自动学习发送方地址）

"""

        text.insert(tk.END, help_text)
        text.config(state=tk.DISABLED)

        # Close button
        btn_frame = ttk.Frame(dlg, padding="10")
        btn_frame.pack(fill=tk.X)
        ttk.Button(btn_frame, text="关闭", command=dlg.destroy).pack(side=tk.RIGHT)

    def toggle_auto_save(self):
        self.auto_save_enabled = self.auto_save_var.get()
        if self.auto_save_enabled and not self.auto_save_file:
            self.select_save_file()
            if not self.auto_save_file:
                self.auto_save_var.set(False)
                self.auto_save_enabled = False

    def select_save_file(self):
        filepath = filedialog.asksaveasfilename(
            title="Select Save File",
            defaultextension=".txt",
            filetypes=[("Text files", "*.txt"), ("Hex files", "*.hex"), ("All files", "*.*")]
        )
        if filepath:
            self.auto_save_file = filepath
            self.save_file_label.config(text=filepath, foreground="black")
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
            self.extracted_save_label.config(text=filepath, foreground="black")
            # Reset buffer and frame counter
            self._extracted_save_buffer = []
            self._extracted_save_frame_start = self.frame_count
            # Check if file already has content (header already written)
            try:
                import os
                self._extracted_header_written = os.path.exists(filepath) and os.path.getsize(filepath) > 0
            except Exception as e:
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
                print(f"RX error: {e}")
                break

    def udp_receive_loop(self):
        while not self.stop_event.is_set() and self.udp_socket:
            try:
                data, addr = self.udp_socket.recvfrom(65536)
                if data:
                    self.rx_buffer.extend(data)
                    self._save_pending.extend(data)
                    self.rx_total_bytes += len(data)
                    # Update remote address if not set
                    if not self.udp_remote_addr:
                        self.udp_remote_addr = addr
                    self._process_buffer()
            except socket.timeout:
                continue
            except OSError:
                # Socket closed or invalid - normal during disconnect
                if self.stop_event.is_set() or not self.udp_socket:
                    break
                else:
                    raise
            except Exception as e:
                if not self.stop_event.is_set():
                    print(f"UDP RX error: {e}")
                break

    def _process_buffer(self):
        """处理接收缓冲区：查找帧，提取多路数值"""
        header_len = len(self.frame_header)
        # Read channel configs (in case user changed them)
        channel_configs = self._read_channel_configs_from_ui()
        active_channels = [i for i, cfg in enumerate(channel_configs) if cfg['active']]

        # Calculate sample parameters for multi-sample mode
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
                    del self.rx_buffer[:-(header_len - 1)]
                break
            if idx > 0:
                del self.rx_buffer[:idx]
            if len(self.rx_buffer) < self.frame_length:
                break

            frame = bytes(self.rx_buffer[:self.frame_length])
            del self.rx_buffer[:self.frame_length]

            # Extract values for each sample in the frame
            num_samples = self.samples_per_packet if self.samples_per_packet > 1 else 1

            for s in range(num_samples):
                sample_values = []
                sample_ok = True

                for i, cfg in enumerate(channel_configs):
                    if cfg['active'] and i < self.channel_count:
                        # Calculate offset for this sample
                        if self.samples_per_packet > 1:
                            offset = sample_start + s * sample_size - sample_start
                            # Adjust indices by sample offset
                            adjusted_indices = [idx + s * sample_size for idx in cfg['indices']]
                        else:
                            adjusted_indices = cfg['indices']

                        val = self._extract_value_from_frame(frame, adjusted_indices, cfg['sign_bit'])
                        if val is not None:
                            self.channel_values[i].append(val)
                            sample_values.append(val)
                        else:
                            sample_ok = False

                # Save to extracted data buffer if enabled
                if sample_ok and self.extracted_save_enabled and len(sample_values) == len(active_channels):
                    self._extracted_save_buffer.append((self.frame_count, sample_values))

                self.frame_count += 1

    def _start_plot_update(self):
        if self.is_connected:
            self._update_plot()
            self.root.after(100, self._start_plot_update)

    def _update_plot(self):
        """更新绘图"""
        # Auto save raw data
        if self.auto_save_enabled and self._save_pending and self.auto_save_file:
            data = bytes(self._save_pending)
            self._save_pending.clear()
            hex_str = ' '.join(f'{b:02X}' for b in data)
            try:
                with open(self.auto_save_file, 'a') as f:
                    f.write(hex_str + ' ')
            except Exception as e:
                print(f"Save error: {e}")

        # Save extracted data to CSV
        if self.extracted_save_enabled and self._extracted_save_buffer and self.extracted_save_file:
            buffer = self._extracted_save_buffer
            self._extracted_save_buffer = []
            try:
                with open(self.extracted_save_file, 'a') as f:
                    # Write header if not written yet
                    if not self._extracted_header_written:
                        channel_configs = self._read_channel_configs_from_ui()
                        active_channels = [i for i, cfg in enumerate(channel_configs) if cfg['active']]
                        header = "Frame," + ",".join(f"CH{i+1}" for i in active_channels) + "\n"
                        f.write(header)
                        self._extracted_header_written = True

                    for frame_num, values in buffer:
                        line = f"{frame_num}," + ",".join(str(v) for v in values) + "\n"
                        f.write(line)
            except Exception as e:
                print(f"Extracted save error: {e}")

        # Read channel configs for diff settings
        channel_configs = self._read_channel_configs_from_ui()

        # Update each channel line
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

                # Diff
                if cfg['diff'] and len(vals) >= 2:
                    diffs = self._compute_first_diff(self.channel_values[i])
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

            # Update status with first channel value
            active_channels = [i for i, cfg in enumerate(channel_configs) if cfg['active']]
            if active_channels:
                first = active_channels[0]
                if len(self.channel_values[first]) > 0:
                    last_val = self.channel_values[first][-1]
                    self.status_var.set(
                        f"{'Connected' if self.is_connected else 'Disconnected'} | "
                        f"Frames: {self.frame_count} | CH{first+1}: {last_val} | "
                        f"Active: {len(active_channels)} ch"
                    )

    def _redraw_plot(self):
        """重新绘制图形（用于配置变更后）"""
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
            self._reset_toolbar_home()
        self.canvas.draw_idle()

    def _reset_toolbar_home(self):
        """重置toolbar的Home视图为当前自动缩放视图"""
        if hasattr(self, 'toolbar') and self.toolbar:
            try:
                self.toolbar._views.clear()
                self.toolbar._positions.clear()
                self.toolbar.push_current()
            except:
                pass

    def clear_plot(self):
        for i in range(self.channel_count):
            self.channel_values[i].clear()
            self.channel_diff_values[i].clear()
            self.channel_lines[i].set_data([], [])
            self.channel_diff_lines[i].set_data([], [])
        self.frame_count = 0
        # Reset extracted save buffer
        self._extracted_save_buffer = []
        self._extracted_save_frame_start = 0
        self.ax.relim()
        self.canvas.draw()
        self.status_var.set("Cleared")

    def _on_mouse_wheel(self, event):
        """鼠标滚轮缩放"""
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
        """解析十六进制字符串，支持多路"""
        # Apply frame config
        try:
            self.apply_frame_config()
        except:
            pass

        # Read channel configs from UI
        channel_configs = self._read_channel_configs_from_ui()

        try:
            hex_str = re.sub(r'[\s\r\n]+', '', hex_str)
            data = bytes.fromhex(hex_str)
        except ValueError as e:
            messagebox.showerror("Parse Error", f"Invalid hex data: {e}")
            return

        # Find frames
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

        # Calculate sample parameters for multi-sample mode
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

        # Extract values for each active channel (supporting multi-sample)
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

        # Update plot data
        for i in range(self.channel_count):
            vals = channel_results[i]
            if vals:
                self.channel_values[i] = deque(vals, maxlen=max(self.max_points, len(vals)))
            else:
                self.channel_values[i].clear()
            self.channel_diff_values[i].clear()

        self.frame_count = len(frames) * num_samples

        # Update plot lines
        for i in range(self.channel_count):
            cfg = channel_configs[i] if i < len(channel_configs) else {'active': False, 'diff': False}
            visible = self.channel_visible[i] if i < len(self.channel_visible) else True
            vals = list(self.channel_values[i])

            if cfg['active'] and vals and visible:
                x = list(range(len(vals)))
                self.channel_lines[i].set_data(x, vals)
                self.channel_lines[i].set_visible(True)

                if cfg['diff'] and len(vals) >= 2:
                    diffs = self._compute_first_diff(self.channel_values[i])
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
        self._reset_toolbar_home()
        self.canvas.draw_idle()

        # Build result message
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
    root = tk.Tk()
    app = FrameAnalyzer(root)
    root.mainloop()
