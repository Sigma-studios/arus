// Shared by every platform's view: colours, the server address, the loading
// and error screens.

import 'dart:io';

import 'package:flutter/material.dart';

const defaultUrl = 'https://arus.online-app.xyz';
const background = Color(0xFF0B0B0C);
const foreground = Color(0xFFA0A0A8);
const accent = Color(0xFF8B5CF6);

/// The server to open: `--url <url>`, else ARUS_URL (desktop), else arus.
String startUrl(List<String> args) {
  final i = args.indexOf('--url');
  if (i >= 0 && i + 1 < args.length) return args[i + 1];
  final env = Platform.environment['ARUS_URL'];
  if (env != null && env.isNotEmpty) return env;
  return defaultUrl;
}

/// Shown over the webview until the page has loaded, or instead of it.
Widget overlay({required bool failed, required String url, required String error, required VoidCallback retry}) {
  final Widget content = failed
      ? message(
          'Could not reach arus',
          'Check your internet connection.\n$url · $error',
          action: FilledButton(onPressed: retry, child: const Text('Try again')),
        )
      : const SizedBox(
          width: 28,
          height: 28,
          child: CircularProgressIndicator(strokeWidth: 2.5, color: accent),
        );
  return ColoredBox(color: background, child: Center(child: content));
}

Widget message(String title, String body, {Widget? action}) {
  return Padding(
    padding: const EdgeInsets.all(24),
    child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(title,
            textAlign: TextAlign.center,
            style: const TextStyle(color: Colors.white, fontSize: 20, fontWeight: FontWeight.w600)),
        const SizedBox(height: 10),
        SelectableText(body,
            textAlign: TextAlign.center, style: const TextStyle(color: foreground, height: 1.5)),
        if (action != null) ...[const SizedBox(height: 20), action],
      ],
    ),
  );
}
