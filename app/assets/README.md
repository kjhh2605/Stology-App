# Stology Local 앱 아이콘

확정 시안: 01 인디고 S 리본 (#4F46E5).

- `StologyLocal.png`: 확정 S 리본의 앱 적용용 PNG (1254×1254). 회색 바깥 여백·그림자·내부 타일 프레임을 제거한 흰색 전체 배경이다.
- `StologyLocal.icns`: macOS 앱 번들용 아이콘, 16–1024px.
- 재생성: `bash app/scripts/build-icon.sh`
- 앱 빌드 시 `scripts/build.sh`가 아이콘을 Resources에 복사하고 `CFBundleIconFile`로 지정한다.
- `icon-edit-prompt.txt`: 내장 imagegen으로 앱 적용용 이미지를 정리할 때 사용한 프롬프트. 최대 기본 해상도를 요청했으며 실제 생성 원본은 1254×1254다. ICNS는 1024px까지 원본에서 축소 생성한다.
