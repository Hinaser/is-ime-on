# 性能

常駐ソフトなので、CPUとメモリへの影響を実測して残しておく。計測手順は末尾。

## CPU

| 状態 | CPU時間 |
|---|---|
| 無操作 | 0.0ms / 60秒 |
| 通常操作(IMEオフ) | ほぼ0。100〜500ms間隔でIME状態を読むだけ(1回数十µs) |
| 日本語入力中(インジケーター表示) | 1コアの0.3%程度 |

## メモリ

常駐プロセスはタスクマネージャーの「メモリ」列(プライベートワーキングセット)で約5MB。
v0.2.0 のリリースビルドを既定設定で起動して測った値:

| 指標 | インジケーター非表示 | インジケーター描画中 |
|---|---|---|
| プライベートワーキングセット(タスクマネージャーの「メモリ」) | 4.5MB | 5.5〜5.6MB |
| プライベート(コミット済み、`PrivateMemorySize64`) | 6.4〜6.6MB | 7.6〜9.4MB |
| ワーキングセット(Direct2D など共有DLLのページを含む) | 21〜27MB | 29〜31MB |

「非表示」は既定設定(IMEオフではインジケーターを出さない)のまま起動・無操作・メモ帳への
入力を各30秒続けた間の値で、ほぼ一定。「描画中」はIMEオフでもインジケーターを出す設定にして
同じ操作をした値で、描画を始めると Direct2D の分だけ増え、その後は横ばい。範囲は2回の計測の幅。
ワーキングセットは他プロセスと共有している DLL のページも数えるため、このアプリ単体の消費量としては
プライベートワーキングセットを使う。

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

## パフォーマンスログ(オプトイン)

設定画面の「診断 → パフォーマンスログを記録する」をオンにすると、
`%APPDATA%\IsImeOn\perf.log` に1分ごとのサマリーを1行ずつ書く。既定はオフで、
オフの間は計測点ごとにフラグを1回読むだけ。オンの間は無操作でも1分に1回の
追記が発生する。オフにしたとき・終了したときは途中までの集計と `stop` 行を書く。

```text
2026-09-27 02:26:43 start v0.2.0 poll=100ms shape=badge position=above
2026-09-27 02:26:43 slow uia 50.4ms (Notepad.exe)
2026-09-27 02:27:43 interval=60s cpu=140.6ms ws=29.6MB private=9.0MB polls=589 events=58 ime=589/avg 417us/max 25.8ms caret=589/avg 12us/max 215us uia=3/avg 25.9ms/max 50.4ms render=54/avg 1.9ms/max 3.8ms
```

| 項目 | 意味 |
|---|---|
| `interval` | 区間の長さ(秒、切り捨て)。ハングしたアプリ相手にポーラーが待たされると60を超えることがある |
| `cpu` | 区間中のプロセスCPU時間(カーネル+ユーザー)。常駐プロセスのみで、設定ウィンドウ(別プロセス)は含まない |
| `ws` / `private` | 常駐プロセスのワーキングセット / プライベートメモリ |
| `polls` / `events` | ポーラーの読み取り回数 / WinEvent フックの通知回数 |
| `ime` `caret` `uia` `render` | IME状態の読み取り / システムキャレット取得 / UI Automation でのキャレット取得 / オーバーレイ再描画。回数・平均・最大 |
| `slow` | 50ms を超えた1回(その場で追記)。その時の前面アプリの exe 名つき |

1MB を超えたら `perf.old.log` に回す(1世代のみ)。ログは外部へ送信しない。
不具合報告のときに添付してもらう想定。例の値はデバッグビルドのもの。

## 計測手順

CPU時間は TotalProcessorTime の差分。コア数に依存しない値になる。

```powershell
$p = Get-Process IsImeOn | Select-Object -First 1
$c0 = $p.TotalProcessorTime
Start-Sleep -Seconds 60
$p.Refresh()
"CPU: {0:N1} ms / 60s" -f ($p.TotalProcessorTime - $c0).TotalMilliseconds
```

メモリ(`WorkingSetPrivate` がタスクマネージャーの「メモリ」列と同じ値):

```powershell
$p = Get-Process IsImeOn | Select-Object -First 1
$p | Select-Object WorkingSet64, PrivateMemorySize64
(Get-CimInstance Win32_PerfRawData_PerfProc_Process -Filter "IDProcess=$($p.Id)").WorkingSetPrivate
```

計測環境は Windows 11 Pro (26200)。「日本語入力中」はメモ帳をひらがなモードにして
キャレットを30秒動かし続けたときの値。UI Automation 経由になるアプリ(Chrome等)では
キャレット取得1回あたり数msかかることがあり、その分だけ増える。
