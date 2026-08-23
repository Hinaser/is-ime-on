# 性能

常駐ソフトなので、CPUとメモリへの影響を実測して残しておく。計測手順は末尾。

## CPU

| 状態 | CPU時間 |
|---|---|
| 無操作 | 0.0ms / 60秒 |
| 通常操作(IMEオフ) | ほぼ0。100〜500ms間隔でIME状態を読むだけ(1回数十µs) |
| 日本語入力中(インジケーター表示) | 1コアの0.3%程度 |

## メモリ

常駐プロセスは約7MB(Private)。

設定ウィンドウは別プロセスとして起動し、閉じるとプロセスごと消える。
常駐側のメモリは設定画面の開閉で変化しない。

## 電力まわりの設計

ポーリングだけに頼らない。フォアグラウンド切替・フォーカス移動・キャレット移動の
WinEvent フックで反応し、ポーリングは取りこぼし対策として回す。無操作が10秒続いたら
間隔を100msから500msに落とし、イベントが来たら即復帰する。CPUを起こす回数は
無操作時2回/秒、操作中10回/秒+入力イベント分。

表示しないモード(既定ではIMEオフ)ではキャレット取得(UI Automation含む)を丸ごと
省く。キャレットが動かない間は再描画もしない。

設定ウィンドウの描画は DirectX 12。OpenGL は NVIDIA ドライバが SwapBuffers で
ビジーウェイトし1コアを使い切るため使っていない。

## 計測手順

CPU時間は TotalProcessorTime の差分。コア数に依存しない値になる。

```powershell
$p = Get-Process IsImeOn | Select-Object -First 1
$c0 = $p.TotalProcessorTime
Start-Sleep -Seconds 60
$p.Refresh()
"CPU: {0:N1} ms / 60s" -f ($p.TotalProcessorTime - $c0).TotalMilliseconds
```

メモリ:

```powershell
Get-Process IsImeOn | Select-Object WorkingSet64, PrivateMemorySize64
```

計測環境は Windows 11 Pro (26200)。「日本語入力中」はメモ帳をひらがなモードにして
キャレットを30秒動かし続けたときの値。UI Automation 経由になるアプリ(Chrome等)では
キャレット取得1回あたり数msかかることがあり、その分だけ増える。
