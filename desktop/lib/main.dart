// arus for Windows: the hosted arus web app in its own window, rendered by
// Edge WebView2. Logins, choreographies and music all stay on the server; the
// window keeps its own cookies (so you stay logged in) under %APPDATA%.
//
// Another server: `arus.exe --url http://localhost:8080` or ARUS_URL.

import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';
import 'package:webview_windows/webview_windows.dart';

const defaultUrl = 'https://arus.online-app.xyz';
const background = Color(0xFF0B0B0C);
const foreground = Color(0xFFA0A0A8);
const accent = Color(0xFF8B5CF6);

String startUrl(List<String> args) {
  final i = args.indexOf('--url');
  if (i >= 0 && i + 1 < args.length) return args[i + 1];
  final env = Platform.environment['ARUS_URL'];
  if (env != null && env.isNotEmpty) return env;
  return defaultUrl;
}

void main(List<String> args) {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(ArusApp(url: startUrl(args)));
}

class ArusApp extends StatelessWidget {
  const ArusApp({super.key, required this.url});

  final String url;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'arus',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        brightness: Brightness.dark,
        scaffoldBackgroundColor: background,
        colorScheme: const ColorScheme.dark(primary: accent, surface: background),
      ),
      home: ArusWindow(url: url),
    );
  }
}

class ArusWindow extends StatefulWidget {
  const ArusWindow({super.key, required this.url});

  final String url;

  @override
  State<ArusWindow> createState() => _ArusWindowState();
}

enum _Phase { starting, loading, ready, failed, noRuntime }

class _ArusWindowState extends State<ArusWindow> {
  final _controller = WebviewController();
  final _subscriptions = <StreamSubscription>[];
  _Phase _phase = _Phase.starting;
  String _error = '';

  @override
  void initState() {
    super.initState();
    _start();
  }

  Future<void> _start() async {
    try {
      if (await WebviewController.getWebViewVersion() == null) {
        setState(() => _phase = _Phase.noRuntime);
        return;
      }
      // Cookies and cache live in %APPDATA%\...\arus_desktop\WebView2, not next
      // to the exe (which may sit in a read-only folder).
      final support = await getApplicationSupportDirectory();
      await WebviewController.initializeEnvironment(
        userDataPath: '${support.path}${Platform.pathSeparator}WebView2',
      );
      await _controller.initialize();
      await _controller.setBackgroundColor(background);
      await _controller.setPopupWindowPolicy(WebviewPopupWindowPolicy.allow);
      _subscriptions
        ..add(_controller.loadingState.listen((state) {
          if (state == LoadingState.navigationCompleted && _phase == _Phase.loading) {
            setState(() => _phase = _Phase.ready);
          }
        }))
        ..add(_controller.onLoadError.listen((status) {
          setState(() {
            _phase = _Phase.failed;
            _error = status.name;
          });
        }));
      setState(() => _phase = _Phase.loading);
      await _controller.loadUrl(widget.url);
    } catch (e) {
      setState(() {
        _phase = _Phase.failed;
        _error = '$e';
      });
    }
  }

  Future<void> _retry() async {
    setState(() => _phase = _Phase.loading);
    await _controller.loadUrl(widget.url);
  }

  Future<WebviewPermissionDecision> _onPermission(
      String url, WebviewPermissionKind kind, bool isUserInitiated) async {
    // Pasting into the editor; nothing else (camera, location, …) is used.
    return kind == WebviewPermissionKind.clipboardRead
        ? WebviewPermissionDecision.allow
        : WebviewPermissionDecision.deny;
  }

  @override
  void dispose() {
    for (final s in _subscriptions) {
      s.cancel();
    }
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final webview = _controller.value.isInitialized
        ? Webview(_controller, permissionRequested: _onPermission)
        : const SizedBox.expand();
    return Scaffold(
      body: Stack(
        children: [
          Positioned.fill(child: webview),
          if (_phase != _Phase.ready) Positioned.fill(child: _overlay()),
        ],
      ),
    );
  }

  Widget _overlay() {
    final Widget content;
    switch (_phase) {
      case _Phase.starting:
      case _Phase.loading:
      case _Phase.ready:
        content = const SizedBox(
          width: 28,
          height: 28,
          child: CircularProgressIndicator(strokeWidth: 2.5, color: accent),
        );
      case _Phase.failed:
        content = _message(
          'Could not reach arus',
          'Check your internet connection.\n${widget.url} · $_error',
          action: FilledButton(onPressed: _retry, child: const Text('Try again')),
        );
      case _Phase.noRuntime:
        content = _message(
          'Microsoft Edge WebView2 is missing',
          'arus needs the WebView2 Runtime. Install it from\n'
              'https://go.microsoft.com/fwlink/p/?LinkId=2124703\nthen start arus again.',
        );
    }
    return ColoredBox(color: background, child: Center(child: content));
  }

  Widget _message(String title, String body, {Widget? action}) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(title,
            style: const TextStyle(color: Colors.white, fontSize: 20, fontWeight: FontWeight.w600)),
        const SizedBox(height: 10),
        SelectableText(body,
            textAlign: TextAlign.center, style: const TextStyle(color: foreground, height: 1.5)),
        if (action != null) ...[const SizedBox(height: 20), action],
      ],
    );
  }
}
