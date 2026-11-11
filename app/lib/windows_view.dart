// Windows: Edge WebView2. Downloads and file pickers work natively.

import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';
import 'package:webview_windows/webview_windows.dart';

import 'common.dart';

class WindowsView extends StatefulWidget {
  const WindowsView({super.key, required this.url});

  final String url;

  @override
  State<WindowsView> createState() => _WindowsViewState();
}

enum _Phase { starting, loading, ready, failed, noRuntime }

class _WindowsViewState extends State<WindowsView> {
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
      // Cookies and cache live in %APPDATA%\...\arus\WebView2, not next
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
          if (_phase == _Phase.noRuntime)
            Positioned.fill(
              child: ColoredBox(
                color: background,
                child: Center(
                  child: message(
                    'Microsoft Edge WebView2 is missing',
                    'arus needs the WebView2 Runtime. Install it from\n'
                        'https://go.microsoft.com/fwlink/p/?LinkId=2124703\nthen start arus again.',
                  ),
                ),
              ),
            )
          else if (_phase != _Phase.ready)
            Positioned.fill(
              child: overlay(failed: _phase == _Phase.failed, url: widget.url, error: _error, retry: _retry),
            ),
        ],
      ),
    );
  }
}
