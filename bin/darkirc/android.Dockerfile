FROM ubuntu

RUN apt update
RUN apt install -yq openjdk-19-jre-headless openjdk-19-jdk-headless
RUN apt install -yq wget unzip cmake file

RUN cd /tmp/ && \
    wget -O install-rustup.sh https://sh.rustup.rs && \
    sh install-rustup.sh -yq --default-toolchain none && \
    rm install-rustup.sh
ENV PATH "${PATH}:/root/.cargo/bin/"
RUN rustup default nightly
RUN rustup target add aarch64-linux-android
#RUN rustup target add armv7-linux-androideabi
#RUN rustup target add i686-linux-android
#RUN rustup target add x86_64-linux-android

# Install Android SDK
ENV ANDROID_HOME /opt/android-sdk/
RUN mkdir ${ANDROID_HOME} && \
    cd ${ANDROID_HOME} && \
    wget -O cmdline-tools.zip -q https://dl.google.com/android/repository/commandlinetools-linux-10406996_latest.zip && \
    unzip cmdline-tools.zip && \
    rm cmdline-tools.zip
# Required by SDKManager
RUN cd ${ANDROID_HOME}/cmdline-tools/ && \
    mkdir latest && \
    mv bin lib latest
RUN yes | ${ANDROID_HOME}/cmdline-tools/latest/bin/sdkmanager --licenses
RUN ${ANDROID_HOME}/cmdline-tools/latest/bin/sdkmanager "platform-tools"
RUN ${ANDROID_HOME}/cmdline-tools/latest/bin/sdkmanager "platforms;android-34"
RUN ${ANDROID_HOME}/cmdline-tools/latest/bin/sdkmanager "ndk;25.2.9519653"
RUN ${ANDROID_HOME}/cmdline-tools/latest/bin/sdkmanager "build-tools;34.0.0"

RUN echo '[target.aarch64-linux-android] \n\
ar = "/opt/android-sdk/ndk/25.2.9519653/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar" \n\
linker = "/opt/android-sdk/ndk/25.2.9519653/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android33-clang" \n\
' > /root/.cargo/config

# Needed by the ring dependency
ENV TARGET_AR /opt/android-sdk/ndk/25.2.9519653/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar
ENV TARGET_CC /opt/android-sdk/ndk/25.2.9519653/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android33-clang

# Make sqlite3
RUN cd /tmp/ && \
    wget -O sqlite.zip https://www.sqlite.org/2023/sqlite-amalgamation-3430000.zip && \
    unzip sqlite.zip && \
    rm sqlite.zip && \
    mv sqlite* sqlite && \
    cd sqlite && \
    mkdir build && \
    mv *.c *.h build/ && \
    mkdir jni && \
    echo '\
APP_ABI := arm64-v8a \n\
APP_CPPFLAGS += -fexceptions -frtti \n\
APP_STL := c++_shared' > jni/Application.mk && \
    echo '\
LOCAL_PATH := $(call my-dir) \n\
include $(CLEAR_VARS) \n\
LOCAL_MODULE            := sqlite3-a \n\
LOCAL_MODULE_FILENAME   := libsqlite3 \n\
LOCAL_SRC_FILES         := ../build/sqlite3.c \n\
LOCAL_C_INCLUDES        := ../build \n\
LOCAL_EXPORT_C_INCLUDES := ../build \n\
LOCAL_CFLAGS            := -DSQLITE_THREADSAFE=1 \n\
include $(BUILD_STATIC_LIBRARY)' > jni/Android.mk && \
    /opt/android-sdk/ndk/25.2.9519653/ndk-build
ENV RUSTFLAGS "-L/tmp/sqlite/obj/local/arm64-v8a/"

# Make directory for user code
RUN mkdir /root/src
WORKDIR /root/src/bin/darkirc/

